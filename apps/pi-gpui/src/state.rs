//! Durable GUI state and a fast session index.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, ErrorCode, TransactionBehavior, params};

use crate::{
    projects::{DraftSession, Registry},
    protocol::{PromptImage, PromptMode},
    sessions::{SessionSummary, UsageSummary},
};

const SCHEMA_VERSION: i64 = 3;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct StateStore {
    connection: Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueuedPrompt {
    pub id: i64,
    pub target: String,
    pub project: PathBuf,
    pub session: Option<PathBuf>,
    pub mode: PromptMode,
    pub message: String,
    pub images: Vec<PromptImage>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ComposerRecord {
    pub target: String,
    pub text: String,
    pub cursor: usize,
    pub selection_start: usize,
    pub selection_end: usize,
    pub history: Vec<String>,
}

impl StateStore {
    pub(crate) fn open() -> Result<Self, String> {
        Self::open_at(&state_path()?)
    }

    pub(crate) fn open_at(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("state database has no parent: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
        let mut connection =
            Connection::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
        connection
            .busy_timeout(DATABASE_BUSY_TIMEOUT)
            .map_err(|error| format!("configure database lock wait: {error}"))?;
        enable_wal(&connection)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|error| format!("enable foreign keys: {error}"))?;
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS meta (
                   key TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS projects (
                   path TEXT PRIMARY KEY,
                   added_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS drafts (
                   id TEXT PRIMARY KEY,
                   project TEXT NOT NULL,
                   created_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS sessions (
                   path TEXT PRIMARY KEY,
                   id TEXT NOT NULL,
                   project TEXT NOT NULL,
                   title TEXT NOT NULL,
                   first_user_message TEXT NOT NULL,
                   timestamp TEXT NOT NULL,
                   parent_session TEXT,
                   modified_ms INTEGER NOT NULL,
                   file_size INTEGER NOT NULL,
                   message_count INTEGER NOT NULL,
                   input_tokens INTEGER NOT NULL,
                   output_tokens INTEGER NOT NULL,
                   cache_read_tokens INTEGER NOT NULL,
                   cache_write_tokens INTEGER NOT NULL,
                   total_tokens INTEGER NOT NULL,
                   cost_micros INTEGER NOT NULL,
                   search_text TEXT NOT NULL,
                   settled_ms INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS sessions_modified ON sessions(modified_ms DESC);
                 CREATE INDEX IF NOT EXISTS sessions_parent ON sessions(parent_session);
                 CREATE TABLE IF NOT EXISTS outbox (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   target TEXT NOT NULL,
                   project TEXT NOT NULL,
                   session_path TEXT,
                   mode TEXT NOT NULL,
                   message TEXT NOT NULL,
                   state TEXT NOT NULL DEFAULT 'queued',
                   created_ms INTEGER NOT NULL,
                   error TEXT
                 );
                 CREATE INDEX IF NOT EXISTS outbox_target_state ON outbox(target, state, id);
                 CREATE TABLE IF NOT EXISTS composer_sessions (
                   target TEXT PRIMARY KEY,
                   text TEXT NOT NULL,
                   cursor INTEGER NOT NULL,
                   selection_start INTEGER NOT NULL,
                   selection_end INTEGER NOT NULL,
                   history_json TEXT NOT NULL,
                   updated_ms INTEGER NOT NULL
                 );
                 INSERT OR IGNORE INTO meta(key, value) VALUES('schema_version', '1');
                 COMMIT;",
            )
            .map_err(|error| format!("create GUI state schema: {error}"))?;
        let migration = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start GUI state schema migration: {error}"))?;
        let schema_version = migration
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("read GUI state schema version: {error}"))?;
        match schema_version {
            1 => migration
                .execute_batch(
                    "ALTER TABLE outbox ADD COLUMN images_json TEXT NOT NULL DEFAULT '[]';
                     ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN session_path TEXT;
                     UPDATE meta SET value='3' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 3: {error}"))?,
            2 => migration
                .execute_batch(
                    "ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN session_path TEXT;
                     UPDATE meta SET value='3' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 3: {error}"))?,
            SCHEMA_VERSION => {}
            _ => {
                return Err(format!(
                    "GUI state schema {schema_version} is not supported by this build"
                ));
            }
        }
        migration
            .execute(
                "UPDATE drafts SET submitted=0 WHERE submitted != 0 AND session_path IS NULL",
                [],
            )
            .map_err(|error| format!("repair submitted drafts without session paths: {error}"))?;
        migration
            .commit()
            .map_err(|error| format!("commit GUI state schema migration: {error}"))?;
        Ok(Self { connection })
    }

    pub(crate) fn load_registry(&self) -> Result<Registry, String> {
        let mut projects = Vec::new();
        let mut statement = self
            .connection
            .prepare("SELECT path FROM projects ORDER BY added_ms, path")
            .map_err(|error| format!("read projects: {error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query projects: {error}"))?;
        for row in rows {
            if let Some(path) = existing_directory(&row.map_err(|error| error.to_string())?) {
                projects.push(path);
            }
        }
        let mut drafts = Vec::new();
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, project, created_ms, submitted, session_path
                   FROM drafts ORDER BY created_ms DESC",
            )
            .map_err(|error| format!("read drafts: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|error| format!("query drafts: {error}"))?;
        for row in rows {
            let (id, project, created_ms, submitted, session_path) =
                row.map_err(|error| error.to_string())?;
            if let Some(project) = existing_directory(&project) {
                drafts.push(DraftSession {
                    id,
                    project,
                    created_ms,
                    submitted,
                    session_path: session_path
                        .map(PathBuf::from)
                        .map(|path| crate::sessions::normalize_session_path(&path)),
                });
            }
        }
        Ok(Registry { projects, drafts })
    }

    pub(crate) fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        if let Some(draft) = registry
            .drafts
            .iter()
            .find(|draft| draft.submitted && draft.session_path.is_none())
        {
            return Err(format!(
                "save draft {}: submitted draft has no session path",
                draft.id
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("start registry update: {error}"))?;
        transaction
            .execute("DELETE FROM projects", [])
            .map_err(|error| format!("clear projects: {error}"))?;
        transaction
            .execute("DELETE FROM drafts", [])
            .map_err(|error| format!("clear drafts: {error}"))?;
        let now = now_ms();
        for (index, project) in registry.projects.iter().enumerate() {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO projects(path, added_ms) VALUES(?1, ?2)",
                    params![project.to_string_lossy(), now.saturating_add(index as u64)],
                )
                .map_err(|error| format!("save project {}: {error}", project.display()))?;
        }
        for draft in &registry.drafts {
            let session_path = draft
                .session_path
                .as_ref()
                .map(|path| crate::sessions::normalize_session_path(path));
            transaction
                .execute(
                    "INSERT INTO drafts(id, project, created_ms, submitted, session_path)
                     VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![
                        draft.id,
                        draft.project.to_string_lossy(),
                        draft.created_ms,
                        draft.submitted,
                        session_path.as_ref().map(|path| path.to_string_lossy())
                    ],
                )
                .map_err(|error| format!("save draft {}: {error}", draft.id))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit registry update: {error}"))
    }

    pub(crate) fn cached_sessions(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        let escaped = query
            .to_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let needle = format!("%{escaped}%");
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, path, project, title, first_user_message, timestamp,
                        parent_session, modified_ms, message_count, input_tokens,
                        output_tokens, cache_read_tokens, cache_write_tokens,
                        total_tokens, cost_micros, search_text, settled_ms IS NOT NULL
                   FROM sessions
                  WHERE ?1 = '%%' OR search_text LIKE ?1 ESCAPE '\\'
                  ORDER BY modified_ms DESC, timestamp DESC",
            )
            .map_err(|error| format!("prepare cached sessions: {error}"))?;
        let rows = statement
            .query_map([needle], row_to_session)
            .map_err(|error| format!("query cached sessions: {error}"))?;
        rows.map(|row| row.map_err(|error| format!("decode cached session: {error}")))
            .collect()
    }

    pub(crate) fn replace_sessions(&mut self, sessions: &[SessionSummary]) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("start session index update: {error}"))?;
        let known = sessions
            .iter()
            .map(|session| session.path.to_string_lossy().into_owned())
            .collect::<HashSet<_>>();
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO sessions(
                       path, id, project, title, first_user_message, timestamp,
                       parent_session, modified_ms, file_size, message_count,
                       input_tokens, output_tokens, cache_read_tokens,
                       cache_write_tokens, total_tokens, cost_micros, search_text
                     ) VALUES(
                       ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       ?11, ?12, ?13, ?14, ?15, ?16, ?17
                     ) ON CONFLICT(path) DO UPDATE SET
                       id=excluded.id, project=excluded.project, title=excluded.title,
                       first_user_message=excluded.first_user_message,
                       timestamp=excluded.timestamp, parent_session=excluded.parent_session,
                       modified_ms=excluded.modified_ms, file_size=excluded.file_size,
                       message_count=excluded.message_count, input_tokens=excluded.input_tokens,
                       output_tokens=excluded.output_tokens,
                       cache_read_tokens=excluded.cache_read_tokens,
                       cache_write_tokens=excluded.cache_write_tokens,
                       total_tokens=excluded.total_tokens, cost_micros=excluded.cost_micros,
                       search_text=excluded.search_text",
                )
                .map_err(|error| format!("prepare session index update: {error}"))?;
            for session in sessions {
                let size = std::fs::metadata(&session.path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                statement
                    .execute(params![
                        session.path.to_string_lossy(),
                        session.id,
                        session.project.to_string_lossy(),
                        session.title,
                        session.first_user_message,
                        session.timestamp,
                        session.parent_session,
                        system_time_ms(session.modified),
                        size,
                        usize_to_u64(session.message_count),
                        session.usage.input,
                        session.usage.output,
                        session.usage.cache_read,
                        session.usage.cache_write,
                        session.usage.total,
                        session.usage.cost_micros,
                        session.search_text(),
                    ])
                    .map_err(|error| {
                        format!("index session {}: {error}", session.path.display())
                    })?;
            }
        }
        let mut paths = transaction
            .prepare("SELECT path FROM sessions")
            .map_err(|error| format!("read indexed paths: {error}"))?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query indexed paths: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("decode indexed paths: {error}"))?;
        paths.retain(|path| !known.contains(path));
        for path in paths {
            transaction
                .execute("DELETE FROM sessions WHERE path=?1", [path])
                .map_err(|error| format!("remove stale session: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session index: {error}"))
    }

    pub(crate) fn set_settled(&self, path: &Path, settled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE sessions SET settled_ms=?2 WHERE path=?1",
                params![
                    path.to_string_lossy(),
                    settled.then_some(now_ms()).map(u64_to_i64)
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("update settled state for {}: {error}", path.display()))
    }

    pub(crate) fn enqueue_prompt(
        &self,
        target: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        let images_json = serde_json::to_string(images)
            .map_err(|error| format!("encode prompt images: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO outbox(
                   target, project, session_path, mode, message, images_json, created_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    target,
                    project.to_string_lossy(),
                    session.map(|path| path.to_string_lossy()),
                    prompt_mode(mode),
                    message,
                    images_json,
                    now_ms(),
                ],
            )
            .map_err(|error| format!("queue prompt: {error}"))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub(crate) fn queued_prompts(&self) -> Result<Vec<QueuedPrompt>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, target, project, session_path, mode, message, images_json
                   FROM outbox WHERE state='queued' ORDER BY id",
            )
            .map_err(|error| format!("prepare prompt queue: {error}"))?;
        statement
            .query_map([], |row| {
                let mode = row.get::<_, String>(4)?;
                let images_json = row.get::<_, String>(6)?;
                let images = serde_json::from_str(&images_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(QueuedPrompt {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    project: PathBuf::from(row.get::<_, String>(2)?),
                    session: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                    mode: parse_prompt_mode(&mode),
                    message: row.get(5)?,
                    images,
                })
            })
            .map_err(|error| format!("query prompt queue: {error}"))?
            .map(|row| row.map_err(|error| format!("decode queued prompt: {error}")))
            .collect()
    }

    pub(crate) fn complete_prompt(&self, id: i64) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM outbox WHERE id=?1", [id])
            .map(|_| ())
            .map_err(|error| format!("complete queued prompt {id}: {error}"))
    }

    pub(crate) fn begin_prompt(&self, id: i64) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE outbox SET state='sending', error=NULL WHERE id=?1 AND state='queued'",
                [id],
            )
            .map_err(|error| format!("start queued prompt {id}: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err(format!("queued prompt {id} is no longer ready to send"))
        }
    }

    pub(crate) fn fail_prompt(&self, id: i64, error: &str) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE outbox SET state='failed', error=?2 WHERE id=?1",
                params![id, error],
            )
            .map(|_| ())
            .map_err(|db_error| format!("fail queued prompt {id}: {db_error}"))
    }

    pub(crate) fn load_composer_sessions(&self) -> Result<Vec<ComposerRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT target, text, cursor, selection_start, selection_end, history_json
                   FROM composer_sessions",
            )
            .map_err(|error| format!("prepare composer sessions: {error}"))?;
        statement
            .query_map([], |row| {
                let history_json = row.get::<_, String>(5)?;
                let history = serde_json::from_str(&history_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(ComposerRecord {
                    target: row.get(0)?,
                    text: row.get(1)?,
                    cursor: row.get::<_, u64>(2)?.try_into().unwrap_or(usize::MAX),
                    selection_start: row.get::<_, u64>(3)?.try_into().unwrap_or(usize::MAX),
                    selection_end: row.get::<_, u64>(4)?.try_into().unwrap_or(usize::MAX),
                    history,
                })
            })
            .map_err(|error| format!("query composer sessions: {error}"))?
            .map(|row| row.map_err(|error| format!("decode composer session: {error}")))
            .collect()
    }

    pub(crate) fn save_composer_session(&self, record: &ComposerRecord) -> Result<(), String> {
        let history_json = serde_json::to_string(&record.history)
            .map_err(|error| format!("encode composer history: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO composer_sessions(
                   target, text, cursor, selection_start, selection_end, history_json, updated_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(target) DO UPDATE SET
                   text=excluded.text,
                   cursor=excluded.cursor,
                   selection_start=excluded.selection_start,
                   selection_end=excluded.selection_end,
                   history_json=excluded.history_json,
                   updated_ms=excluded.updated_ms",
                params![
                    &record.target,
                    &record.text,
                    usize_to_i64(record.cursor),
                    usize_to_i64(record.selection_start),
                    usize_to_i64(record.selection_end),
                    history_json,
                    u64_to_i64(now_ms()),
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("save composer session {}: {error}", record.target))
    }

    pub(crate) fn delete_composer_session(&self, target: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM composer_sessions WHERE target=?1", [target])
            .map(|_| ())
            .map_err(|error| format!("delete composer session {target}: {error}"))
    }
}

pub(crate) fn state_path() -> Result<PathBuf, String> {
    let root = if let Some(root) = std::env::var_os("PI_CODING_AGENT_DIR") {
        PathBuf::from(root)
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set and PI_CODING_AGENT_DIR is not set".to_owned())?
            .join(".pi/agent")
    };
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .map_err(|error| format!("resolve GUI state directory: {error}"))?
            .join(root)
    };
    Ok(root.join("gui-state.sqlite3"))
}

fn enable_wal(connection: &Connection) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match connection.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                ) && started.elapsed() < DATABASE_BUSY_TIMEOUT =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("enable WAL: {error}")),
        }
    }
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary::from_cached(
        row.get(0)?,
        PathBuf::from(row.get::<_, String>(1)?),
        PathBuf::from(row.get::<_, String>(2)?),
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        UNIX_EPOCH + std::time::Duration::from_millis(row.get::<_, u64>(7)?),
        row.get::<_, u64>(8)?.try_into().unwrap_or(usize::MAX),
        UsageSummary {
            input: row.get(9)?,
            output: row.get(10)?,
            cache_read: row.get(11)?,
            cache_write: row.get(12)?,
            total: row.get(13)?,
            cost_micros: row.get(14)?,
        },
        row.get(16)?,
        row.get(15)?,
    ))
}

fn existing_directory(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path).canonicalize().ok()?;
    path.is_dir().then_some(path)
}

fn now_ms() -> u64 {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn u64_to_i64(value: u64) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}

const fn prompt_mode(mode: PromptMode) -> &'static str {
    match mode {
        PromptMode::Normal => "normal",
        PromptMode::Steer => "steer",
        PromptMode::FollowUp => "follow_up",
    }
}

fn parse_prompt_mode(mode: &str) -> PromptMode {
    match mode {
        "steer" => PromptMode::Steer,
        "follow_up" => PromptMode::FollowUp,
        _ => PromptMode::Normal,
    }
}

fn usize_to_i64(value: usize) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}
