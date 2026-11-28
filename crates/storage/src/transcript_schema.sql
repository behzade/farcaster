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
CREATE INDEX IF NOT EXISTS transcript_prompt_receipts ON session_events(session_id,json_extract(body,'$.submissionId'))
  WHERE json_extract(body,'$.type') IN ('prompt_delivery_receipt','prompt_delivery_resolution');
CREATE INDEX IF NOT EXISTS transcript_prompt_dispatch ON session_events(session_id,json_extract(body,'$.submissionId'))
  WHERE json_extract(body,'$.type')='prompt_dispatch';
CREATE INDEX IF NOT EXISTS transcript_user_ordinals ON transcript_messages(session_id,ordinal) WHERE user_message=1;
CREATE INDEX IF NOT EXISTS transcript_linked_ordinals ON transcript_messages(session_id,ordinal) WHERE submission_id IS NOT NULL OR inherited_fields=1;

CREATE TABLE IF NOT EXISTS transcript_viewports (
    session_id INTEGER PRIMARY KEY REFERENCES transcript_documents(session_id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision INTEGER NOT NULL,
    event_seq INTEGER NOT NULL,
    from_beginning INTEGER NOT NULL,
    items BLOB
);
CREATE INDEX IF NOT EXISTS transcript_prompt_events
  ON session_events(session_id,seq) WHERE json_extract(body,'$.type') IN
   ('accepted_prompt','prompt_presentation','prompt_delivery_receipt','prompt_delivery_resolution');

INSERT OR IGNORE INTO meta(key,value)
SELECT 'transcript_viewport_backfill','1' WHERE EXISTS(SELECT 1 FROM transcript_documents);
