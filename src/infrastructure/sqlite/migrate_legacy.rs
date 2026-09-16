use super::*;

pub(super) fn migrate_to_v11(
    migration: &Transaction<'_>,
    mut schema_version: i64,
) -> Result<(), String> {
    migration
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
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
                 CREATE TABLE IF NOT EXISTS composer_sessions (
                   target TEXT PRIMARY KEY,
                   text TEXT NOT NULL,
                   cursor INTEGER NOT NULL,
                   selection_start INTEGER NOT NULL,
                   selection_end INTEGER NOT NULL,
                   history_json TEXT NOT NULL,
                   updated_ms INTEGER NOT NULL
                 );
                 INSERT OR IGNORE INTO meta(key, value) VALUES('schema_version', '1');",
        )
        .map_err(|error| format!("create GUI state schema: {error}"))?;
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
        5..=11 => {}
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

    Ok(())
}
