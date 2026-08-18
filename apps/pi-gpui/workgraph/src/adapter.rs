use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _, Transaction, TransactionBehavior, params};

use crate::{
    contract::{
        Dependency, EditResult, IdempotencyReceipt, Issue, IssueStatus, Note, ProjectRecordId,
        SessionLink,
    },
    core::{Persistence, PersistenceError, TransactionMode, WorkGraphTransaction},
};

const SCHEMA_VERSION: &str = "2";

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
                "SELECT fingerprint, result_json FROM wg_idempotency WHERE key=?1",
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
                "INSERT INTO wg_idempotency(key, fingerprint, result_json, created_ms) VALUES(?1, ?2, ?3, ?4)",
                params![key, fingerprint, result, created_at],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn ensure_project(&mut self, project: &str) -> Result<ProjectRecordId, PersistenceError> {
        self.inner
            .execute(
                "INSERT OR IGNORE INTO wg_projects(path) VALUES(?1)",
                [project],
            )
            .map_err(error)?;
        self.inner
            .query_row(
                "SELECT id FROM wg_projects WHERE path=?1",
                [project],
                |row| row.get(0).map(ProjectRecordId::from_storage),
            )
            .map_err(error)
    }

    fn project_id(&self, project: &str) -> Result<Option<ProjectRecordId>, PersistenceError> {
        self.inner
            .query_row(
                "SELECT id FROM wg_projects WHERE path=?1",
                [project],
                |row| row.get(0).map(ProjectRecordId::from_storage),
            )
            .optional()
            .map_err(error)
    }

    fn next_issue_number(&mut self, project: ProjectRecordId) -> Result<u64, PersistenceError> {
        self.inner
            .query_row(
                "UPDATE wg_projects SET next_issue_number=next_issue_number+1 WHERE id=?1 RETURNING next_issue_number-1",
                [project.as_storage()],
                |row| row.get(0),
            )
            .map_err(error)
    }

    fn insert_issue(
        &mut self,
        project: ProjectRecordId,
        issue: &Issue,
    ) -> Result<(), PersistenceError> {
        self.inner
            .execute(
                "INSERT INTO wg_issues(project_id, number, title, body, status, priority, version, created_ms, updated_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![project.as_storage(), issue.number, issue.title, issue.body, issue.status.as_str(), issue.priority, issue.version, issue.created_at, issue.updated_at],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn issue(
        &self,
        project_path: &str,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Option<Issue>, PersistenceError> {
        self.inner
            .query_row(
                "SELECT number, title, body, status, priority, version, created_ms, updated_ms FROM wg_issues WHERE project_id=?1 AND number=?2",
                params![project.as_storage(), number],
                |row| crate::adapter_rows::issue(row, project_path),
            )
            .optional()
            .map_err(error)
    }

    fn issues(
        &self,
        project_path: &str,
        project: ProjectRecordId,
        status: Option<IssueStatus>,
    ) -> Result<Vec<Issue>, PersistenceError> {
        let mut statement = self.inner.prepare(
            "SELECT number, title, body, status, priority, version, created_ms, updated_ms FROM wg_issues WHERE project_id=?1 AND (?2 IS NULL OR status=?2) ORDER BY priority, created_ms, number",
        ).map_err(error)?;
        let rows = statement
            .query_map(
                params![project.as_storage(), status.map(IssueStatus::as_str)],
                |row| crate::adapter_rows::issue(row, project_path),
            )
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)
    }

    fn set_status(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        status: IssueStatus,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError> {
        self.inner
            .execute(
                "UPDATE wg_issues SET status=?3, version=version+1, updated_ms=?4 WHERE project_id=?1 AND number=?2 AND (?5 IS NULL OR version=?5)",
                params![project.as_storage(), number, status.as_str(), updated_at, expected_version],
            )
            .map(|changed| changed != 0)
            .map_err(error)
    }

    fn bump_version(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError> {
        self.inner
            .execute(
                "UPDATE wg_issues SET version=version+1, updated_ms=?3 WHERE project_id=?1 AND number=?2 AND (?4 IS NULL OR version=?4)",
                params![project.as_storage(), number, updated_at, expected_version],
            )
            .map(|changed| changed != 0)
            .map_err(error)
    }

    fn insert_note(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        body: &str,
        created_at: i64,
    ) -> Result<Note, PersistenceError> {
        self.inner
            .execute(
                "INSERT INTO wg_notes(project_id, issue_number, body, created_ms) VALUES(?1, ?2, ?3, ?4)",
                params![project.as_storage(), number, body, created_at],
            )
            .map_err(error)?;
        Ok(Note {
            id: self.inner.last_insert_rowid(),
            issue_number: number,
            body: body.to_owned(),
            created_at,
        })
    }

    fn notes(&self, project: ProjectRecordId, number: u64) -> Result<Vec<Note>, PersistenceError> {
        let mut statement = self
            .inner
            .prepare("SELECT id, body, created_ms FROM wg_notes WHERE project_id=?1 AND issue_number=?2 ORDER BY created_ms, id")
            .map_err(error)?;
        let rows = statement
            .query_map(params![project.as_storage(), number], |row| {
                Ok(Note {
                    id: row.get(0)?,
                    issue_number: number,
                    body: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)
    }

    fn dependencies(
        &self,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Vec<u64>, PersistenceError> {
        let mut statement = self
            .inner
            .prepare("SELECT depends_on_number FROM wg_dependencies WHERE project_id=?1 AND issue_number=?2 ORDER BY depends_on_number")
            .map_err(error)?;
        let rows = statement
            .query_map(params![project.as_storage(), number], |row| row.get(0))
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)
    }

    fn all_dependencies(
        &self,
        project: ProjectRecordId,
    ) -> Result<Vec<Dependency>, PersistenceError> {
        let mut statement = self
            .inner
            .prepare("SELECT issue_number, depends_on_number FROM wg_dependencies WHERE project_id=?1 ORDER BY issue_number, depends_on_number")
            .map_err(error)?;
        let rows = statement
            .query_map([project.as_storage()], |row| {
                Ok(Dependency {
                    issue_number: row.get(0)?,
                    depends_on: row.get(1)?,
                })
            })
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)
    }

    fn dependency_reaches(
        &self,
        project: ProjectRecordId,
        from: u64,
        target: u64,
    ) -> Result<bool, PersistenceError> {
        self.inner
            .query_row(
                "WITH RECURSIVE descendants(number) AS (SELECT depends_on_number FROM wg_dependencies WHERE project_id=?1 AND issue_number=?2 UNION SELECT d.depends_on_number FROM wg_dependencies d JOIN descendants x ON d.issue_number=x.number WHERE d.project_id=?1) SELECT 1 FROM descendants WHERE number=?3 LIMIT 1",
                params![project.as_storage(), from, target],
                |_| Ok(()),
            )
            .optional()
            .map(|value| value.is_some())
            .map_err(error)
    }

    fn add_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError> {
        self.inner
            .execute(
                "INSERT OR IGNORE INTO wg_dependencies(project_id, issue_number, depends_on_number) VALUES(?1, ?2, ?3)",
                params![project.as_storage(), number, depends_on],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn remove_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError> {
        self.inner
            .execute(
                "DELETE FROM wg_dependencies WHERE project_id=?1 AND issue_number=?2 AND depends_on_number=?3",
                params![project.as_storage(), number, depends_on],
            )
            .map(|_| ())
            .map_err(error)
    }

    fn session_link(
        &self,
        project: ProjectRecordId,
        session_id: &str,
    ) -> Result<Option<SessionLink>, PersistenceError> {
        self.inner
            .query_row(
                "SELECT session_id, session_path, issue_number, linked_ms FROM wg_session_links WHERE project_id=?1 AND session_id=?2",
                params![project.as_storage(), session_id],
                crate::adapter_rows::session_link,
            )
            .optional()
            .map_err(error)
    }

    fn session_links(
        &self,
        project: ProjectRecordId,
        issue_number: Option<u64>,
    ) -> Result<Vec<SessionLink>, PersistenceError> {
        let mut statement = self.inner.prepare(
            "SELECT session_id, session_path, issue_number, linked_ms FROM wg_session_links WHERE project_id=?1 AND (?2 IS NULL OR issue_number=?2) ORDER BY linked_ms, session_id",
        ).map_err(error)?;
        let rows = statement
            .query_map(params![project.as_storage(), issue_number], crate::adapter_rows::session_link)
            .map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)
    }

    fn upsert_session_link(
        &mut self,
        project: ProjectRecordId,
        link: &SessionLink,
    ) -> Result<(), PersistenceError> {
        self.inner.execute(
            "INSERT INTO wg_session_links(project_id, session_id, session_path, issue_number, linked_ms) VALUES(?1, ?2, ?3, ?4, ?5) ON CONFLICT(project_id, session_id) DO UPDATE SET session_path=excluded.session_path, issue_number=excluded.issue_number, linked_ms=excluded.linked_ms",
            params![project.as_storage(), link.session_id, link.session_path, link.issue_number, link.linked_at],
        ).map(|_| ()).map_err(error)
    }

    fn remove_session_link(
        &mut self,
        project: ProjectRecordId,
        session_id: &str,
    ) -> Result<(), PersistenceError> {
        self.inner.execute(
            "DELETE FROM wg_session_links WHERE project_id=?1 AND session_id=?2",
            params![project.as_storage(), session_id],
        ).map(|_| ()).map_err(error)
    }

    fn commit(self) -> Result<(), PersistenceError> {
        self.inner.commit().map_err(error)
    }
}

fn migrate(connection: &Connection) -> Result<(), PersistenceError> {
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let migration = (|| {
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
            if version.as_deref().is_some_and(|version| version != "1" && version != SCHEMA_VERSION) {
                return Err(PersistenceError::new(format!(
                    "work graph schema {} is not supported",
                    version.unwrap_or_default()
                )));
            }
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS wg_projects (
               id INTEGER PRIMARY KEY,
               path TEXT NOT NULL UNIQUE,
               next_issue_number INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS wg_issues (
               project_id INTEGER NOT NULL,
               number INTEGER NOT NULL,
               title TEXT NOT NULL,
               body TEXT NOT NULL,
               status TEXT NOT NULL,
               priority INTEGER NOT NULL,
               version INTEGER NOT NULL,
               created_ms INTEGER NOT NULL,
               updated_ms INTEGER NOT NULL,
               PRIMARY KEY(project_id, number),
               FOREIGN KEY(project_id) REFERENCES wg_projects(id) ON DELETE CASCADE,
               CHECK(length(title) BETWEEN 1 AND 512),
               CHECK(length(body) <= 1000000),
               CHECK(priority >= 0),
               CHECK(version > 0)
             );
             CREATE INDEX IF NOT EXISTS wg_issues_plan ON wg_issues(project_id, status, priority, created_ms, number);
             CREATE TABLE IF NOT EXISTS wg_dependencies (
               project_id INTEGER NOT NULL,
               issue_number INTEGER NOT NULL,
               depends_on_number INTEGER NOT NULL,
               PRIMARY KEY(project_id, issue_number, depends_on_number),
               FOREIGN KEY(project_id, issue_number) REFERENCES wg_issues(project_id, number) ON DELETE CASCADE,
               FOREIGN KEY(project_id, depends_on_number) REFERENCES wg_issues(project_id, number) ON DELETE RESTRICT,
               CHECK(issue_number != depends_on_number)
             );
             CREATE TABLE IF NOT EXISTS wg_notes (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               project_id INTEGER NOT NULL,
               issue_number INTEGER NOT NULL,
               body TEXT NOT NULL,
               created_ms INTEGER NOT NULL,
               FOREIGN KEY(project_id, issue_number) REFERENCES wg_issues(project_id, number) ON DELETE CASCADE,
               CHECK(length(body) BETWEEN 1 AND 100000)
             );
             CREATE TABLE IF NOT EXISTS wg_idempotency (
               key TEXT PRIMARY KEY,
               fingerprint TEXT NOT NULL,
               result_json TEXT NOT NULL,
               created_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS wg_session_links (
               project_id INTEGER NOT NULL,
               session_id TEXT NOT NULL,
               session_path TEXT NOT NULL,
               issue_number INTEGER NOT NULL,
               linked_ms INTEGER NOT NULL,
               PRIMARY KEY(project_id, session_id),
               FOREIGN KEY(project_id, issue_number) REFERENCES wg_issues(project_id, number) ON DELETE CASCADE,
               CHECK(length(session_id) BETWEEN 1 AND 256),
               CHECK(length(session_path) BETWEEN 1 AND 4096)
             );
             CREATE INDEX IF NOT EXISTS wg_session_links_issue ON wg_session_links(project_id, issue_number);
             INSERT INTO meta(key, value) VALUES('workgraph_schema_version', '2')
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
