use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "ALTER TABLE outbox RENAME TO outbox_v19;
         CREATE TABLE outbox (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           submission_event_seq INTEGER,
           mode TEXT NOT NULL,
           message TEXT NOT NULL,
           display_message TEXT,
           invocation TEXT,
           images_json TEXT NOT NULL DEFAULT '[]',
           provider TEXT,
           model TEXT,
           effort TEXT,
           service_tier TEXT,
           state TEXT NOT NULL DEFAULT 'pending'
             CHECK (state IN ('pending', 'acked', 'cancelled')),
           error TEXT,
           created_ms INTEGER NOT NULL
         );
         INSERT INTO outbox(
           id, session_id, submission_event_seq, mode, message, display_message,
           invocation, images_json, provider, model, effort, service_tier, state,
           error, created_ms
         )
         SELECT id, session_id, submission_event_seq, mode, message, display_message,
                invocation, images_json, provider, model, effort, service_tier,
                CASE
                  WHEN state IN ('acked', 'cancelled') THEN state
                  ELSE 'pending'
                END,
                error, created_ms
           FROM outbox_v19;
         DROP TABLE outbox_v19;
         CREATE INDEX outbox_session_state ON outbox(session_id, state, id);",
    )
    .map_err(|error| format!("normalize outbox delivery states: {error}"))
}

#[cfg(test)]
#[path = "migrate_v20_tests.rs"]
mod tests;
