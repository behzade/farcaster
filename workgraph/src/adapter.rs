use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};

use crate::{
    contract::{EditResult, IdempotencyReceipt, StoredProject},
    core::{Persistence, PersistenceError, TransactionMode, WorkGraphTransaction},
};

const SCHEMA_VERSION: &str = "3";

pub struct SqliteAdapter {
    connection: Connection,
}

impl SqliteAdapter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        let connection = Connection::open(path).map_err(error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(10))
            .map_err(error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(error)?;
        migrate(&connection)?;
        Ok(Self { connection })
    }
}

impl Persistence for SqliteAdapter {
    type Transaction<'a> = SqliteTransaction<'a>;

    fn begin(&mut self, mode: TransactionMode) -> Result<Self::Transaction<'_>, PersistenceError> {
        let behavior = match mode {
            TransactionMode::Read => TransactionBehavior::Deferred,
            TransactionMode::Write => TransactionBehavior::Immediate,
        };
        self.connection
            .transaction_with_behavior(behavior)
            .map(|inner| SqliteTransaction { inner })
            .map_err(error)
    }
}

pub struct SqliteTransaction<'a> {
    inner: Transaction<'a>,
}

impl WorkGraphTransaction for SqliteTransaction<'_> {
    fn idempotency_receipt(
        &self,
        key: &str,
    ) -> Result<Option<IdempotencyReceipt>, PersistenceError> {
        let stored = self
            .inner
            .query_row(
                "SELECT fingerprint, result_json FROM wg_plan_receipts WHERE key=?1",
                [key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(error)?;
        stored
            .map(|(fingerprint, result)| {
                serde_json::from_str(&result)
                    .map(|result| IdempotencyReceipt {
                        fingerprint,
                        result,
                    })
                    .map_err(error)
            })
            .transpose()
    }

    fn record_idempotency(
        &mut self,
        key: &str,
        fingerprint: &str,
        result: &EditResult,
        created_at: i64,
    ) -> Result<(), PersistenceError> {
        let result = serde_json::to_string(result).map_err(error)?;
        self.inner
            .execute(
                "INSERT INTO wg_plan_receipts(key, fingerprint, result_json, created_ms) VALUES(?1, ?2, ?3, ?4)",
                params![key, fingerprint, result, created_at],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn project(&self, project: &str) -> Result<Option<StoredProject>, PersistenceError> {
        let value = self
            .inner
            .query_row(
                "SELECT data_json FROM wg_plan_store WHERE project=?1",
                [project],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?;
        value
            .map(|value| serde_json::from_str(&value).map_err(error))
            .transpose()
    }

    fn save_project(
        &mut self,
        project: &str,
        value: &StoredProject,
        updated_at: i64,
    ) -> Result<(), PersistenceError> {
        let value = serde_json::to_string(value).map_err(error)?;
        self.inner
            .execute(
                "INSERT INTO wg_plan_store(project, data_json, updated_ms) VALUES(?1, ?2, ?3)
                 ON CONFLICT(project) DO UPDATE SET data_json=excluded.data_json, updated_ms=excluded.updated_ms",
                params![project, value, updated_at],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn commit(self) -> Result<(), PersistenceError> {
        self.inner.commit().map_err(error)
    }
}

fn migrate(connection: &Connection) -> Result<(), PersistenceError> {
    let has_meta = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(error)?;
    if has_meta {
        let version = connection
            .query_row(
                "SELECT value FROM meta WHERE key='workgraph_schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?;
        if version
            .as_deref()
            .is_some_and(|version| !matches!(version, "1" | "2" | SCHEMA_VERSION))
        {
            return Err(PersistenceError::new(format!(
                "work graph schema {} is not supported",
                version.unwrap_or_default()
            )));
        }
    }

    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let migration = (|| {
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS wg_plan_store (
                   project TEXT PRIMARY KEY,
                   data_json TEXT NOT NULL,
                   updated_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS wg_plan_receipts (
                   key TEXT PRIMARY KEY,
                   fingerprint TEXT NOT NULL,
                   result_json TEXT NOT NULL,
                   created_ms INTEGER NOT NULL
                 );
                 INSERT INTO meta(key, value) VALUES('workgraph_schema_version', '3')
                   ON CONFLICT(key) DO UPDATE SET value=excluded.value;",
            )
            .map_err(error)?;
        connection.execute_batch("COMMIT").map_err(error)
    })();
    if migration.is_err() {
        let _rollback = connection.execute_batch("ROLLBACK");
    }
    migration
}

fn error(value: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::new(value.to_string())
}
