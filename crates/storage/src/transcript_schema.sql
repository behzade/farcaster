CREATE TABLE IF NOT EXISTS transcript_documents (
  session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL DEFAULT 0,
  complete INTEGER NOT NULL DEFAULT 0,
  item_count INTEGER NOT NULL DEFAULT 0,
  updated_ms INTEGER NOT NULL DEFAULT 0,
  metadata TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS transcript_messages (
  session_id INTEGER NOT NULL REFERENCES transcript_documents(session_id) ON DELETE CASCADE ON UPDATE CASCADE,
  item_key TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  user_message INTEGER NOT NULL,
  inherited_fields INTEGER NOT NULL DEFAULT 0,
  source_digest BLOB NOT NULL,
  native_key TEXT,
  submission_id TEXT,
  body TEXT NOT NULL,
  images TEXT NOT NULL,
  PRIMARY KEY (session_id, item_key),
  UNIQUE (session_id, ordinal)
);
CREATE INDEX IF NOT EXISTS transcript_native ON transcript_messages(session_id, native_key) WHERE native_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS transcript_submission ON transcript_messages(session_id, submission_id) WHERE submission_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS transcript_accepted_prompt ON session_events(session_id,CAST(json_extract(body,'$.submissionId') AS TEXT))
  WHERE json_extract(body,'$.type')='accepted_prompt';
CREATE INDEX IF NOT EXISTS transcript_user_ordinals ON transcript_messages(session_id,ordinal) WHERE user_message=1;
CREATE INDEX IF NOT EXISTS transcript_linked_ordinals ON transcript_messages(session_id,ordinal) WHERE submission_id IS NOT NULL OR inherited_fields=1;
