use super::*;

impl StateStore {
    pub(crate) fn open() -> Result<Self, String> {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.open_total");
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
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.open_at");
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
        {
            let _timing =
                crate::app::infrastructure::performance::StartupTiming::new("db.enable_wal");
            enable_wal(&connection)?;
        }
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|error| format!("enable foreign keys: {error}"))?;
        let schema_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.ensure_schema");
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
        drop(schema_timing);
        let migration_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.migrate");
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
            5 | 6 | 7 | 8 | 9 | 10 | SCHEMA_VERSION => {}
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
            schema_version = 10;
        }
        if schema_version == 10 {
            migration
                .execute_batch(
                    "ALTER TABLE outbox ADD COLUMN display_message TEXT;
                     ALTER TABLE outbox ADD COLUMN invocation TEXT;
                     CREATE TABLE prompt_presentations (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       session_path TEXT NOT NULL,
                       resolved_message TEXT NOT NULL,
                       display_message TEXT NOT NULL,
                       invocation TEXT NOT NULL,
                       created_ms INTEGER NOT NULL
                     );
                     CREATE INDEX prompt_presentations_session
                       ON prompt_presentations(session_path, created_ms, id);
                     UPDATE meta SET value='11' WHERE key='schema_version';",
                )
                .map_err(|error| format!("migrate GUI state schema to 11: {error}"))?;
        }
        migration
            .commit()
            .map_err(|error| format!("commit GUI state schema migration: {error}"))?;
        drop(migration_timing);
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
