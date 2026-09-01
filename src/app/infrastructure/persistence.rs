use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};

use crate::{
    agents::QueuedPrompt,
    projects::{DraftSession, Registry},
    protocol::{PromptImage, PromptMode},
    sessions::{SessionSummary, UsageSummary},
};

const SCHEMA_VERSION: i64 = 10;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTIVE_IMPORT_WINDOW: Duration = Duration::from_secs(3 * 60 * 60);
const REPOSITORY_BACKEND_PREFERENCES_KEY: &str = "repository_backend_preferences";
const NETWORK_PROXY_KEY: &str = "network_proxy";
const LEGACY_PI_GPUI_IMPORT_KEY: &str = "legacy_pi_gpui_state_imported";
const REPOSITORY_BACKENDS: [&str; 3] = ["auto", "git", "jj"];

pub(crate) struct StateStore {
    connection: Connection,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct WindowPlacement {
    pub bounds: [f32; 4],
    pub display_uuid: Option<String>,
    pub display_origin: [f32; 2],
    pub state: WindowState,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) enum WindowState {
    Windowed,
    Maximized,
    Fullscreen,
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
        let path = state_path()?;
        let mut store = Self::open_at(&path)?;
        if let Some(legacy) = legacy_pi_gpui_state_path()
            && legacy != path
            && legacy.is_file()
        {
            store.import_legacy_pi_gpui_state(&legacy)?;
        }
        Ok(store)
    }

    pub(crate) fn open_at(path: &Path) -> Result<Self, String> {
        let _timing = crate::app::infrastructure::performance::OperationTiming::new(
            crate::app::infrastructure::performance::OperationKind::StateDatabase,
            1,
        );
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
        let mut schema_version = migration
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
                     ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN provisional_title TEXT;
                     UPDATE meta SET value='5' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 5: {error}"))?,
            2 => migration
                .execute_batch(
                    "ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN session_path TEXT;
                     ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN provisional_title TEXT;
                     UPDATE meta SET value='5' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 5: {error}"))?,
            3 => migration
                .execute_batch(
                    "ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0;
                     ALTER TABLE drafts ADD COLUMN provisional_title TEXT;
                     UPDATE meta SET value='5' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 5: {error}"))?,
            4 => migration
                .execute_batch(
                    "ALTER TABLE drafts ADD COLUMN provisional_title TEXT;
                     UPDATE meta SET value='5' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 5: {error}"))?,
            5 | 6 | 7 | 8 | 9 | SCHEMA_VERSION => {}
            _ => {
                return Err(format!(
                    "GUI state schema {schema_version} is not supported by this build"
                ));
            }
        }
        if schema_version < 5 {
            schema_version = 5;
        }
        if schema_version == 5 {
            migration
                .execute_batch(
                    "CREATE TABLE app_sessions (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       draft_id TEXT UNIQUE,
                       session_path TEXT UNIQUE,
                       created_ms INTEGER NOT NULL
                     );
                     ALTER TABLE drafts ADD COLUMN app_session_id INTEGER;
                     ALTER TABLE sessions ADD COLUMN app_session_id INTEGER;
                     INSERT OR IGNORE INTO app_sessions(session_path, created_ms)
                       SELECT path, modified_ms FROM sessions ORDER BY modified_ms, path;
                     UPDATE app_sessions
                        SET draft_id=(
                          SELECT drafts.id FROM drafts
                           WHERE drafts.session_path=app_sessions.session_path
                           ORDER BY drafts.created_ms LIMIT 1
                        )
                      WHERE draft_id IS NULL;
                     INSERT OR IGNORE INTO app_sessions(draft_id, session_path, created_ms)
                       SELECT id, session_path, created_ms FROM drafts ORDER BY created_ms, id;
                     UPDATE sessions
                        SET app_session_id=(
                          SELECT id FROM app_sessions WHERE session_path=sessions.path
                        );
                     UPDATE drafts
                        SET app_session_id=COALESCE(
                          (SELECT id FROM app_sessions WHERE draft_id=drafts.id),
                          (SELECT id FROM app_sessions WHERE session_path=drafts.session_path)
                        );
                     CREATE UNIQUE INDEX drafts_app_session_id ON drafts(app_session_id);
                     CREATE UNIQUE INDEX sessions_app_session_id ON sessions(app_session_id);
                     UPDATE meta SET value='6' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 6: {error}"))?;
            schema_version = 6;
        }
        if schema_version == 6 {
            migration
                .execute_batch("UPDATE meta SET value='7' WHERE key='schema_version';")
                .map_err(|error| format!("migrate GUI state schema to 7: {error}"))?;
            schema_version = 7;
        }
        if schema_version == 7 {
            migration
                .execute_batch(
                    "ALTER TABLE sessions ADD COLUMN harness TEXT NOT NULL DEFAULT 'pi';
                     UPDATE meta SET value='8' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 8: {error}"))?;
            schema_version = 8;
        }
        if schema_version == 8 {
            migration
                .execute_batch(
                    "ALTER TABLE drafts ADD COLUMN harness TEXT NOT NULL DEFAULT 'pi';
                     ALTER TABLE outbox ADD COLUMN harness TEXT NOT NULL DEFAULT 'pi';
                     ALTER TABLE app_sessions ADD COLUMN harness TEXT NOT NULL DEFAULT 'pi';
                     UPDATE meta SET value='9' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 9: {error}"))?;
            schema_version = 9;
        }
        if schema_version == 9 {
            migration
                .execute_batch(
                    "ALTER TABLE app_sessions
                       ADD COLUMN import_classified INTEGER NOT NULL DEFAULT 0;
                     UPDATE sessions
                        SET settled_ms=COALESCE(settled_ms, CAST(unixepoch('now') AS INTEGER) * 1000)
                      WHERE app_session_id IN (
                              SELECT id FROM app_sessions WHERE draft_id IS NULL
                            )
                        AND (
                          is_running=0
                          OR modified_ms < CAST(unixepoch('now') AS INTEGER) * 1000 - 10800000
                        );
                     UPDATE app_sessions SET import_classified=1;
                     UPDATE meta SET value='10' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 10: {error}"))?;
        }
        migration
            .commit()
            .map_err(|error| format!("commit GUI state schema migration: {error}"))?;
        Ok(Self { connection })
    }

    pub(crate) fn import_legacy_pi_gpui_state(&mut self, path: &Path) -> Result<(), String> {
        let imported = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
                [LEGACY_PI_GPUI_IMPORT_KEY],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("check legacy pi-gpui state import: {error}"))?;
        if imported {
            return Ok(());
        }

        let uri = format!("file:{}?mode=ro&immutable=1", path.to_string_lossy());
        self.connection
            .execute("ATTACH DATABASE ?1 AS legacy_pi_gpui", [uri])
            .map_err(|error| format!("attach legacy pi-gpui state {}: {error}", path.display()))?;
        let result = (|| {
            let version = self
                .connection
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM legacy_pi_gpui.meta
                      WHERE key='schema_version'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| format!("read legacy pi-gpui schema version: {error}"))?;
            if !matches!(version, 7 | SCHEMA_VERSION) {
                return Err(format!(
                    "legacy pi-gpui state schema {version} is not supported by this build"
                ));
            }

            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| format!("start legacy pi-gpui state import: {error}"))?;
            transaction
                .execute_batch(
                    "INSERT OR IGNORE INTO projects(path, added_ms)
                       SELECT path, added_ms FROM legacy_pi_gpui.projects;
                     INSERT OR IGNORE INTO sessions(
                       path, id, project, title, first_user_message, timestamp,
                       parent_session, modified_ms, file_size, message_count,
                       input_tokens, output_tokens, cache_read_tokens,
                       cache_write_tokens, total_tokens, cost_micros, search_text,
                       settled_ms, is_running
                     ) SELECT
                       path, id, project, title, first_user_message, timestamp,
                       parent_session, modified_ms, file_size, message_count,
                       input_tokens, output_tokens, cache_read_tokens,
                       cache_write_tokens, total_tokens, cost_micros, search_text,
                       settled_ms, 0
                     FROM legacy_pi_gpui.sessions;
                     UPDATE sessions
                        SET settled_ms=(
                          SELECT settled_ms FROM legacy_pi_gpui.sessions legacy
                           WHERE legacy.path=sessions.path
                        )
                      WHERE settled_ms IS NULL
                        AND EXISTS(
                          SELECT 1 FROM legacy_pi_gpui.sessions legacy
                           WHERE legacy.path=sessions.path AND legacy.settled_ms IS NOT NULL
                        );
                     INSERT INTO composer_sessions(
                       target, text, cursor, selection_start, selection_end,
                       history_json, updated_ms
                     ) SELECT
                       target, text, cursor, selection_start, selection_end,
                       history_json, updated_ms
                     FROM legacy_pi_gpui.composer_sessions WHERE true
                     ON CONFLICT(target) DO UPDATE SET
                       text=excluded.text,
                       cursor=excluded.cursor,
                       selection_start=excluded.selection_start,
                       selection_end=excluded.selection_end,
                       history_json=excluded.history_json,
                       updated_ms=excluded.updated_ms
                     WHERE excluded.updated_ms > composer_sessions.updated_ms;
                     INSERT OR IGNORE INTO meta(key, value)
                       SELECT key, value FROM legacy_pi_gpui.meta
                        WHERE key IN ('excluded_projects', 'repository_backend_preferences');
                     UPDATE meta
                        SET value=(
                          SELECT value FROM legacy_pi_gpui.meta
                           WHERE key='excluded_projects'
                        )
                      WHERE key='excluded_projects' AND value='[]'
                        AND EXISTS(
                          SELECT 1 FROM legacy_pi_gpui.meta
                           WHERE key='excluded_projects'
                        );
                     INSERT INTO meta(key, value) VALUES('legacy_pi_gpui_state_imported', '1');",
                )
                .map_err(|error| format!("import legacy pi-gpui state: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("commit legacy pi-gpui state import: {error}"))
        })();
        let detached = self
            .connection
            .execute("DETACH DATABASE legacy_pi_gpui", [])
            .map(|_| ())
            .map_err(|error| format!("detach legacy pi-gpui state: {error}"));
        result.and(detached)
    }

    pub(crate) fn load_window_placement(&self) -> Result<Option<WindowPlacement>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='window_placement'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load window placement: {error}"))?;
        stored
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|error| format!("decode window placement: {error}"))
            })
            .transpose()
    }

    pub(crate) fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), String> {
        let value = serde_json::to_string(placement)
            .map_err(|error| format!("encode window placement: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('window_placement', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )
            .map_err(|error| format!("save window placement: {error}"))?;
        Ok(())
    }

    pub(crate) fn load_app_session_order(&self) -> Result<Vec<i64>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='app_session_order'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load application session order: {error}"))?;
        stored.map_or_else(
            || Ok(Vec::new()),
            |value| {
                serde_json::from_str(&value)
                    .map_err(|error| format!("decode application session order: {error}"))
            },
        )
    }

    pub(crate) fn save_app_session_order(&self, order: &[i64]) -> Result<(), String> {
        let value = serde_json::to_string(order)
            .map_err(|error| format!("encode application session order: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('app_session_order', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )
            .map(|_| ())
            .map_err(|error| format!("save application session order: {error}"))
    }

    pub(crate) fn load_network_proxy(&self) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [NETWORK_PROXY_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load network proxy: {error}"))
    }

    pub(crate) fn save_network_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
            self.connection
                .execute(
                    "INSERT INTO meta(key, value) VALUES(?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![NETWORK_PROXY_KEY, proxy],
                )
                .map(|_| ())
                .map_err(|error| format!("save network proxy: {error}"))
        } else {
            self.connection
                .execute("DELETE FROM meta WHERE key=?1", [NETWORK_PROXY_KEY])
                .map(|_| ())
                .map_err(|error| format!("clear network proxy: {error}"))
        }
    }

    pub(crate) fn load_repository_backend_preferences(
        &self,
    ) -> Result<BTreeMap<PathBuf, String>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [REPOSITORY_BACKEND_PREFERENCES_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load repository backend preferences: {error}"))?;
        let preferences = stored.map_or_else(
            || Ok(BTreeMap::new()),
            |value| {
                serde_json::from_str::<BTreeMap<PathBuf, String>>(&value)
                    .map_err(|error| format!("decode repository backend preferences: {error}"))
            },
        )?;
        validate_repository_backend_preferences(&preferences)?;
        Ok(preferences)
    }

    pub(crate) fn save_repository_backend_preferences(
        &self,
        preferences: &BTreeMap<PathBuf, String>,
    ) -> Result<(), String> {
        validate_repository_backend_preferences(preferences)?;
        let value = serde_json::to_string(preferences)
            .map_err(|error| format!("encode repository backend preferences: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![REPOSITORY_BACKEND_PREFERENCES_KEY, value],
            )
            .map(|_| ())
            .map_err(|error| format!("save repository backend preferences: {error}"))
    }

    pub(crate) fn allocate_app_session_id(
        &mut self,
        draft_id: &str,
        created_ms: u64,
    ) -> Result<i64, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start application session allocation: {error}"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO app_sessions(draft_id, created_ms) VALUES(?1, ?2)",
                params![draft_id, u64_to_i64(created_ms)],
            )
            .map_err(|error| format!("allocate application session for {draft_id}: {error}"))?;
        let id = transaction
            .query_row(
                "SELECT id FROM app_sessions WHERE draft_id=?1",
                [draft_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("read application session for {draft_id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit application session allocation: {error}"))?;
        Ok(id)
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
        let excluded_projects = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='excluded_projects'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load excluded projects: {error}"))?
            .map_or_else(
                || Ok(Vec::new()),
                |value| {
                    serde_json::from_str(&value)
                        .map_err(|error| format!("decode excluded projects: {error}"))
                },
            )?;
        let mut drafts = Vec::new();
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, app_session_id, harness, project, created_ms, submitted, session_path,
                        provisional_title
                   FROM drafts ORDER BY created_ms DESC",
            )
            .map_err(|error| format!("read drafts: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|error| format!("query drafts: {error}"))?;
        for row in rows {
            let (id, app_session_id, harness, project, created_ms, submitted, session_path, title) =
                row.map_err(|error| error.to_string())?;
            if let Some(project) = existing_directory(&project) {
                drafts.push(DraftSession {
                    id,
                    app_session_id,
                    harness,
                    project,
                    created_ms,
                    submitted,
                    session_path: session_path
                        .map(PathBuf::from)
                        .map(|path| crate::sessions::normalize_session_path(&path)),
                    title,
                });
            }
        }
        Ok(Registry {
            projects,
            excluded_projects,
            drafts,
        })
    }

    pub(crate) fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        let excluded_projects = serde_json::to_string(&registry.excluded_projects)
            .map_err(|error| format!("encode excluded projects: {error}"))?;
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
        transaction
            .execute(
                "INSERT INTO meta(key, value) VALUES('excluded_projects', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [excluded_projects],
            )
            .map_err(|error| format!("save excluded projects: {error}"))?;
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
            let app_session_id =
                ensure_draft_app_session(&transaction, draft, session_path.as_deref())
                    .map_err(|error| format!("identify draft {}: {error}", draft.id))?;
            transaction
                .execute(
                    "INSERT INTO drafts(
                       id, app_session_id, harness, project, created_ms, submitted, session_path,
                       provisional_title
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        draft.id,
                        app_session_id,
                        draft.harness,
                        draft.project.to_string_lossy(),
                        draft.created_ms,
                        draft.submitted,
                        session_path.as_ref().map(|path| path.to_string_lossy()),
                        draft.title
                    ],
                )
                .map_err(|error| format!("save draft {}: {error}", draft.id))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit registry update: {error}"))
    }

    pub(crate) fn cached_sessions(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, path, project, title, first_user_message, timestamp,
                        parent_session, modified_ms, message_count, input_tokens,
                        output_tokens, cache_read_tokens, cache_write_tokens,
                        total_tokens, cost_micros, search_text, settled_ms IS NOT NULL,
                        is_running, app_session_id, harness
                   FROM sessions
                  ORDER BY modified_ms DESC, timestamp DESC",
            )
            .map_err(|error| format!("prepare cached sessions: {error}"))?;
        let rows = statement
            .query_map([], row_to_session)
            .map_err(|error| format!("query cached sessions: {error}"))?;
        let sessions = rows
            .map(|row| row.map_err(|error| format!("decode cached session: {error}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crate::sessions::filter_session_tree(sessions, query))
    }

    #[cfg(test)]
    pub(crate) fn replace_sessions(&mut self, sessions: &[SessionSummary]) -> Result<(), String> {
        self.index_sessions(sessions, true)
    }

    pub(crate) fn index_sessions(
        &mut self,
        sessions: &[SessionSummary],
        prune_missing: bool,
    ) -> Result<(), String> {
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
                       cache_write_tokens, total_tokens, cost_micros, search_text,
                       is_running, app_session_id, harness, settled_ms
                     ) VALUES(
                       ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
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
                       search_text=excluded.search_text, is_running=excluded.is_running,
                       app_session_id=excluded.app_session_id, harness=excluded.harness",
                )
                .map_err(|error| format!("prepare session index update: {error}"))?;
            for session in sessions {
                let (app_session_id, classify_import) =
                    ensure_session_app_session(&transaction, session).map_err(|error| {
                        format!("identify session {}: {error}", session.path.display())
                    })?;
                let settled_ms = (session.archived
                    || classify_import && imported_session_is_archived(session, SystemTime::now()))
                .then(|| now_ms());
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
                        session.is_running,
                        app_session_id,
                        session.harness,
                        settled_ms,
                    ])
                    .map_err(|error| {
                        format!("index session {}: {error}", session.path.display())
                    })?;
                if classify_import {
                    transaction
                        .execute(
                            "UPDATE app_sessions SET import_classified=1 WHERE id=?1",
                            [app_session_id],
                        )
                        .map_err(|error| {
                            format!(
                                "classify imported session {}: {error}",
                                session.path.display()
                            )
                        })?;
                }
            }
        }
        if prune_missing {
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
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session index: {error}"))
    }

    pub(crate) fn has_queued_prompts_for(&self, paths: &[PathBuf]) -> Result<bool, String> {
        for path in paths {
            let queued = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM outbox WHERE session_path=?1 AND state='queued')",
                    [path.to_string_lossy()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| format!("check queued prompts for {}: {error}", path.display()))?;
            if queued {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn relocate_session_paths(
        &mut self,
        paths: &[(PathBuf, PathBuf)],
        target_project: &Path,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session path relocation: {error}"))?;
        for (source, target) in paths {
            let source_text = source.to_string_lossy();
            let target_text = target.to_string_lossy();
            let source_target = format!("session:{source_text}");
            let target_target = format!("session:{target_text}");
            transaction
                .execute(
                    "UPDATE app_sessions SET session_path=?2 WHERE session_path=?1",
                    params![source_text, target_text],
                )
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE sessions SET path=?2, project=?3 WHERE path=?1",
                        params![source_text, target_text, target_project.to_string_lossy()],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE drafts SET session_path=?2, project=?3 WHERE session_path=?1",
                        params![source_text, target_text, target_project.to_string_lossy()],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE outbox SET target=?2, project=?3, session_path=?4 WHERE target=?1",
                        params![
                            source_target,
                            target_target,
                            target_project.to_string_lossy(),
                            target_text
                        ],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE composer_sessions SET target=?2 WHERE target=?1",
                        params![source_target, target_target],
                    )
                })
                .map_err(|error| {
                    format!(
                        "relocate session state {} to {}: {error}",
                        source.display(),
                        target.display()
                    )
                })?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session path relocation: {error}"))
    }

    pub(crate) fn delete_session_state(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session state deletion: {error}"))?;
        for path in paths {
            let path_text = path.to_string_lossy();
            let target = format!("session:{path_text}");
            transaction
                .execute("DELETE FROM outbox WHERE session_path=?1", [&path_text])
                .and_then(|_| {
                    transaction.execute("DELETE FROM composer_sessions WHERE target=?1", [&target])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM drafts WHERE session_path=?1", [&path_text])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM sessions WHERE path=?1", [&path_text])
                })
                .and_then(|_| {
                    transaction.execute(
                        "DELETE FROM app_sessions WHERE session_path=?1",
                        [&path_text],
                    )
                })
                .map_err(|error| format!("delete saved state for {}: {error}", path.display()))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session state deletion: {error}"))
    }

    pub(crate) fn set_session_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE sessions SET settled_ms=?2 WHERE path=?1",
                params![
                    path.to_string_lossy(),
                    archived.then_some(now_ms()).map(u64_to_i64)
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("update archived state for {}: {error}", path.display()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_prompt(
        &self,
        target: &str,
        harness: &str,
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
                   target, harness, project, session_path, mode, message, images_json, created_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    target,
                    harness,
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
                "SELECT id, target, harness, project, session_path, mode, message, images_json
                   FROM outbox WHERE state='queued' ORDER BY id",
            )
            .map_err(|error| format!("prepare prompt queue: {error}"))?;
        statement
            .query_map([], |row| {
                let mode = row.get::<_, String>(5)?;
                let images_json = row.get::<_, String>(7)?;
                let images = serde_json::from_str(&images_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(QueuedPrompt {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    harness: row.get(2)?,
                    project: PathBuf::from(row.get::<_, String>(3)?),
                    session: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                    mode: parse_prompt_mode(&mode),
                    message: row.get(6)?,
                    images,
                })
            })
            .map_err(|error| format!("query prompt queue: {error}"))?
            .map(|row| row.map_err(|error| format!("decode queued prompt: {error}")))
            .collect()
    }

    pub(crate) fn complete_prompt(
        &mut self,
        id: i64,
        target: &str,
        session: Option<&Path>,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("start queued prompt completion {id}: {error}"))?;
        if let Some(draft_id) = target.strip_prefix("draft:").filter(|id| !id.is_empty())
            && let Some(session) = session
        {
            let session = crate::sessions::normalize_session_path(session);
            let app_session_id = transaction
                .query_row(
                    "SELECT app_session_id FROM drafts WHERE id=?1",
                    [draft_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| format!("identify queued prompt {id} draft: {error}"))?;
            if let Some(app_session_id) = app_session_id {
                associate_app_session(&transaction, app_session_id, draft_id, &session).map_err(
                    |error| format!("associate queued prompt {id} application session: {error}"),
                )?;
            }
            transaction
                .execute(
                    "UPDATE drafts SET submitted=1, session_path=?2 WHERE id=?1",
                    params![draft_id, session.to_string_lossy()],
                )
                .map_err(|error| {
                    format!("associate queued prompt {id} with its session: {error}")
                })?;
        }
        transaction
            .execute("DELETE FROM outbox WHERE id=?1", [id])
            .map_err(|error| format!("complete queued prompt {id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit queued prompt completion {id}: {error}"))
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
    crate::app::infrastructure::paths::data_dir().map(|root| root.join("state.sqlite3"))
}

fn legacy_pi_gpui_state_path() -> Option<PathBuf> {
    let root = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent")))?;
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().ok()?.join(root)
    };
    Some(root.join("gui-state.sqlite3"))
}

impl crate::access::NetworkSettingsStore for StateStore {
    fn load_proxy(&self) -> Result<Option<String>, String> {
        self.load_network_proxy()
    }

    fn save_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        self.save_network_proxy(proxy)
    }
}

impl crate::agents::PromptStore for StateStore {
    fn has_queued_for(&self, paths: &[PathBuf]) -> Result<bool, String> {
        self.has_queued_prompts_for(paths)
    }

    fn enqueue(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        self.enqueue_prompt(target, harness, project, session, mode, message, images)
    }

    fn queued(&self) -> Result<Vec<QueuedPrompt>, String> {
        self.queued_prompts()
    }

    fn complete(&mut self, id: i64, target: &str, session: Option<&Path>) -> Result<(), String> {
        self.complete_prompt(id, target, session)
    }

    fn begin(&self, id: i64) -> Result<(), String> {
        self.begin_prompt(id)
    }

    fn fail(&self, id: i64, error: &str) -> Result<(), String> {
        self.fail_prompt(id, error)
    }
}

impl crate::sessions::SessionStore for StateStore {
    fn cached(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        self.cached_sessions(query)
    }

    fn index(&mut self, sessions: &[SessionSummary], prune_missing: bool) -> Result<(), String> {
        self.index_sessions(sessions, prune_missing)
    }

    fn relocate(
        &mut self,
        paths: &[(PathBuf, PathBuf)],
        target_project: &Path,
    ) -> Result<(), String> {
        self.relocate_session_paths(paths, target_project)
    }

    fn delete(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        self.delete_session_state(paths)
    }

    fn set_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        self.set_session_archived(path, archived)
    }
}

impl crate::projects::ProjectStore for StateStore {
    fn allocate_session_id(&mut self, draft_id: &str, created_ms: u64) -> Result<i64, String> {
        self.allocate_app_session_id(draft_id, created_ms)
    }

    fn load_registry(&self) -> Result<Registry, String> {
        StateStore::load_registry(self)
    }

    fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        StateStore::save_registry(self, registry)
    }
}

impl crate::repository::PreferenceStore for StateStore {
    fn load(&self) -> Result<BTreeMap<PathBuf, crate::repository::BackendPreference>, String> {
        self.load_repository_backend_preferences()?
            .into_iter()
            .map(|(project, preference)| preference.parse().map(|preference| (project, preference)))
            .collect()
    }

    fn save(
        &self,
        preferences: &BTreeMap<PathBuf, crate::repository::BackendPreference>,
    ) -> Result<(), String> {
        let preferences = preferences
            .iter()
            .map(|(project, preference)| (project.clone(), preference.as_str().to_owned()))
            .collect();
        self.save_repository_backend_preferences(&preferences)
    }
}

fn validate_repository_backend_preferences(
    preferences: &BTreeMap<PathBuf, String>,
) -> Result<(), String> {
    for (project, backend) in preferences {
        if !project.is_absolute() {
            return Err(format!(
                "repository backend preference project path is not absolute: {}",
                project.display()
            ));
        }
        if !REPOSITORY_BACKENDS.contains(&backend.as_str()) {
            return Err(format!(
                "unknown repository backend preference for {}: {backend}",
                project.display()
            ));
        }
    }
    Ok(())
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

fn ensure_draft_app_session(
    transaction: &Transaction<'_>,
    draft: &DraftSession,
    session_path: Option<&Path>,
) -> rusqlite::Result<i64> {
    if draft.app_session_id > 0 {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(id, draft_id, created_ms, harness) VALUES(?1, ?2, ?3, ?4)",
            params![
                draft.app_session_id,
                draft.id,
                u64_to_i64(draft.created_ms),
                draft.harness
            ],
        )?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO app_sessions(draft_id, created_ms, harness) VALUES(?1, ?2, ?3)",
        params![draft.id, u64_to_i64(draft.created_ms), draft.harness],
    )?;
    let app_session_id = transaction.query_row(
        "SELECT id FROM app_sessions WHERE draft_id=?1",
        [&draft.id],
        |row| row.get::<_, i64>(0),
    )?;
    if let Some(path) = session_path {
        associate_app_session(transaction, app_session_id, &draft.id, path)?;
    }
    Ok(app_session_id)
}

fn ensure_session_app_session(
    transaction: &Transaction<'_>,
    session: &SessionSummary,
) -> rusqlite::Result<(i64, bool)> {
    let path = crate::sessions::normalize_session_path(&session.path);
    if let Some((id, classified)) = transaction
        .query_row(
            "SELECT id, import_classified FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?
    {
        return Ok((id, !classified));
    }
    if session.app_session_id > 0 {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(id, session_path, created_ms, harness)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                session.app_session_id,
                path.to_string_lossy(),
                u64_to_i64(system_time_ms(session.modified)),
                session.harness
            ],
        )?;
        transaction.execute(
            "UPDATE app_sessions SET session_path=?2 WHERE id=?1 AND session_path IS NULL",
            params![session.app_session_id, path.to_string_lossy()],
        )?;
    } else {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(session_path, created_ms, harness) VALUES(?1, ?2, ?3)",
            params![
                path.to_string_lossy(),
                u64_to_i64(system_time_ms(session.modified)),
                session.harness
            ],
        )?;
    }
    transaction
        .query_row(
            "SELECT id, import_classified FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        )
        .map(|(id, classified)| (id, !classified))
}

fn imported_session_is_archived(session: &SessionSummary, now: SystemTime) -> bool {
    now.duration_since(session.modified).unwrap_or_default() > ACTIVE_IMPORT_WINDOW
}

fn associate_app_session(
    transaction: &Transaction<'_>,
    app_session_id: i64,
    draft_id: &str,
    session_path: &Path,
) -> rusqlite::Result<()> {
    let path = crate::sessions::normalize_session_path(session_path);
    let existing = transaction
        .query_row(
            "SELECT id FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(existing) = existing.filter(|existing| *existing != app_session_id) {
        transaction.execute(
            "UPDATE sessions SET app_session_id=?2 WHERE app_session_id=?1 OR path=?3",
            params![existing, app_session_id, path.to_string_lossy()],
        )?;
        transaction.execute("DELETE FROM app_sessions WHERE id=?1", [existing])?;
    }
    transaction.execute(
        "UPDATE app_sessions
            SET draft_id=COALESCE(draft_id, ?2), session_path=?3, import_classified=1
          WHERE id=?1",
        params![app_session_id, draft_id, path.to_string_lossy()],
    )?;
    transaction.execute(
        "UPDATE sessions SET app_session_id=?2 WHERE path=?1",
        params![path.to_string_lossy(), app_session_id],
    )?;
    Ok(())
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary::from_cached_for_harness(
        row.get(0)?,
        row.get(19)?,
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
        row.get(17)?,
        row.get(15)?,
    )
    .with_app_session_id(row.get(18)?))
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
