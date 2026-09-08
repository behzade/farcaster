CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE projects (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  added_ms INTEGER NOT NULL,
  deleted_at INTEGER,
  repository_backend TEXT
);
CREATE TABLE sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  harness TEXT NOT NULL,
  locator TEXT,
  backend_id TEXT,
  parent_backend_id TEXT,
  parent_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  client_key TEXT UNIQUE,
  title TEXT NOT NULL DEFAULT '',
  first_user_message TEXT NOT NULL DEFAULT '',
  search_text TEXT NOT NULL DEFAULT '',
  timestamp TEXT,
  modified_ms INTEGER NOT NULL,
  archived_at INTEGER,
  rail_order INTEGER NOT NULL DEFAULT 0,
  record_coverage TEXT NOT NULL DEFAULT 'unloaded'
    CHECK (record_coverage IN ('unloaded', 'partial', 'complete')),
  message_count INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  total_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL,
  submitted INTEGER NOT NULL DEFAULT 0,
  UNIQUE (harness, locator)
);
CREATE TABLE session_models (
  session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  provider TEXT,
  model TEXT,
  effort TEXT,
  service_tier TEXT
);
CREATE TABLE session_events (
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  t INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  body TEXT NOT NULL,
  PRIMARY KEY (session_id, seq)
);
CREATE TABLE worker_families (
  child_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  execution_json TEXT
);
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
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK (state IN ('queued', 'sending', 'failed', 'unknown')),
  error TEXT,
  created_ms INTEGER NOT NULL
);
CREATE TABLE composer_sessions (
  session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  cursor INTEGER NOT NULL,
  selection_start INTEGER NOT NULL,
  selection_end INTEGER NOT NULL,
  history_json TEXT NOT NULL,
  attachments_json TEXT NOT NULL DEFAULT '[]',
  updated_ms INTEGER NOT NULL
);
CREATE TABLE session_ops (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('move', 'delete', 'rename')),
  intent_json TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending', 'done', 'failed')),
  result_json TEXT,
  created_ms INTEGER NOT NULL
);
CREATE TABLE ui_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  window_placement_json TEXT,
  network_proxy TEXT,
  builtin_mcp_enabled INTEGER NOT NULL DEFAULT 1,
  worker_tasks_json TEXT,
  configuration_catalogs_json TEXT,
  session_control_defaults_json TEXT,
  app_session_order_json TEXT
);
CREATE INDEX sessions_project_rail
  ON sessions(project_id, archived_at, rail_order);
CREATE INDEX sessions_parent ON sessions(parent_id);
CREATE INDEX outbox_session_state ON outbox(session_id, state, id);
