use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_turns (
        id TEXT PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        prompt_id TEXT,
        user_ordinal INTEGER,
        started_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS session_turns_session ON session_turns(session_id, prompt_id);
    CREATE TABLE IF NOT EXISTS session_reviews (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        turn_id TEXT REFERENCES session_turns(id),
        artifact TEXT NOT NULL,
        created_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS session_reviews_session ON session_reviews(session_id, seq);
    CREATE INDEX IF NOT EXISTS sessions_native_identity ON sessions(project_id,harness,backend_id);
    INSERT OR IGNORE INTO session_reviews(id,session_id,artifact,created_ms)
      SELECT json_extract(body,'$.submission.artifact.farcaster_review.id'),session_id,
             json_extract(body,'$.submission.artifact'),t FROM session_events
      WHERE json_extract(body,'$.type')='review_submitted'
        AND json_extract(body,'$.submission.artifact.farcaster_review.id') IS NOT NULL;",
    )
    .map_err(|e| format!("migrate durable reviews: {e}"))
}
