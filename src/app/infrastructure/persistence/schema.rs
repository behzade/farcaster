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
        let _schema_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.ensure_schema");
        if schema_version(&connection)? != Some(SCHEMA_VERSION) {
            let migration = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| format!("start GUI state schema migration: {error}"))?;
            match schema_version(&migration)? {
                None => {
                    migration
                        .execute_batch(include_str!("schema.sql"))
                        .map_err(|error| format!("create GUI state schema: {error}"))?;
                    migration
                        .execute("INSERT INTO ui_state(id) VALUES(1)", [])
                        .map_err(|error| format!("initialize GUI settings: {error}"))?;
                }
                Some(SCHEMA_VERSION) => {}
                Some(version @ 1..=11) => {
                    super::migrate_legacy::migrate_to_v11(&migration, version)?;
                    super::migrate_v12::migrate_v11_to_v12(&migration)?;
                }
                Some(version @ (12 | 13)) => {
                    if version == 12 {
                        migration
                            .execute_batch("ALTER TABLE ui_state DROP COLUMN application_modifier;")
                            .map_err(|error| {
                                format!("remove application modifier setting: {error}")
                            })?;
                    }
                    migration
                        .execute_batch("ALTER TABLE composer_sessions ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';")
                        .map_err(|error| format!("add composer attachments: {error}"))?;
                    migrate_v14_to_v15(&migration)?;
                }
                Some(14) => migrate_v14_to_v15(&migration)?,
                Some(version) => {
                    return Err(format!(
                        "GUI state schema {version} is not supported by this build"
                    ));
                }
            }
            migration
                .execute(
                    "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [SCHEMA_VERSION.to_string()],
                )
                .map_err(|error| format!("set GUI state schema version: {error}"))?;
            migration
                .commit()
                .map_err(|error| format!("commit GUI state schema migration: {error}"))?;
        }
        Ok(Self {
            connection,
            image_directory: std::path::absolute(parent.join("images"))
                .map_err(|error| format!("resolve image directory: {error}"))?,
        })
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
            if !matches!(version, 7 | 11 | 12 | 13 | 14 | SCHEMA_VERSION) {
                return Err(format!(
                    "legacy pi-gpui state schema {version} is not supported by this build"
                ));
            }

            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| format!("start legacy pi-gpui state import: {error}"))?;
            super::migrate_v12::import_legacy_pi_gpui(&transaction)?;
            transaction
                .execute(
                    "INSERT INTO meta(key, value) VALUES('legacy_pi_gpui_state_imported', '1')",
                    [],
                )
                .map_err(|error| format!("mark legacy pi-gpui state imported: {error}"))?;
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

fn migrate_v14_to_v15(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "ALTER TABLE outbox RENAME TO outbox_v14;
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
         INSERT INTO outbox(
           id, session_id, submission_event_seq, mode, message, display_message,
           invocation, images_json, provider, model, effort, service_tier, state,
           error, created_ms
         )
         SELECT id, session_id, submission_event_seq, mode, message, display_message,
                invocation, images_json, provider, model, effort, service_tier, state,
                error, created_ms
           FROM outbox_v14;
         DROP TABLE outbox_v14;
         CREATE INDEX outbox_session_state ON outbox(session_id, state, id);",
    )
    .map_err(|error| format!("add interrupted prompt recovery state: {error}"))
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

fn schema_version(connection: &Connection) -> Result<Option<i64>, String> {
    let has_meta: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("inspect GUI state schema: {error}"))?;
    if !has_meta {
        return Ok(None);
    }
    connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .map(Some)
        .map_err(|error| format!("read GUI state schema version: {error}"))
}
