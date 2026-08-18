use std::time::{SystemTime, UNIX_EPOCH};

use crate::contract::{
    EditAction, EditRequest, EditResult, IdempotencyReceipt, Issue, IssueDetail, IssueStatus, Note,
    PlanningView, ProjectRecordId, SearchRequest, SearchResult,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkGraphError {
    #[error("work graph persistence failed")]
    Persistence(#[from] PersistenceError),
    #[error("work graph data could not be encoded")]
    Encoding(#[from] serde_json::Error),
    #[error("project was not found")]
    ProjectNotFound,
    #[error("issue was not found")]
    IssueNotFound,
    #[error("issue changed since it was read")]
    VersionConflict,
    #[error("idempotency key was reused for a different request")]
    IdempotencyConflict,
    #[error("dependency would create a cycle")]
    DependencyCycle,
    #[error("invalid work graph input: {0}")]
    InvalidInput(&'static str),
    #[error("system clock is unavailable")]
    Clock,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PersistenceError {
    message: String,
}

impl PersistenceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionMode {
    Read,
    Write,
}

pub trait Persistence {
    type Transaction<'a>: WorkGraphTransaction
    where
        Self: 'a;

    fn begin(&mut self, mode: TransactionMode) -> Result<Self::Transaction<'_>, PersistenceError>;
}

pub trait WorkGraphTransaction {
    fn idempotency_receipt(
        &self,
        key: &str,
    ) -> Result<Option<IdempotencyReceipt>, PersistenceError>;
    fn record_idempotency(
        &mut self,
        key: &str,
        fingerprint: &str,
        result: &EditResult,
        created_at: i64,
    ) -> Result<(), PersistenceError>;
    fn ensure_project(&mut self, project: &str) -> Result<ProjectRecordId, PersistenceError>;
    fn project_id(&self, project: &str) -> Result<Option<ProjectRecordId>, PersistenceError>;
    fn next_issue_number(&mut self, project: ProjectRecordId) -> Result<u64, PersistenceError>;
    fn insert_issue(
        &mut self,
        project: ProjectRecordId,
        issue: &Issue,
    ) -> Result<(), PersistenceError>;
    fn issue(
        &self,
        project_path: &str,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Option<Issue>, PersistenceError>;
    fn issues(
        &self,
        project_path: &str,
        project: ProjectRecordId,
        status: Option<IssueStatus>,
    ) -> Result<Vec<Issue>, PersistenceError>;
    fn set_status(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        status: IssueStatus,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError>;
    fn bump_version(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError>;
    fn insert_note(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        body: &str,
        created_at: i64,
    ) -> Result<Note, PersistenceError>;
    fn notes(&self, project: ProjectRecordId, number: u64) -> Result<Vec<Note>, PersistenceError>;
    fn dependencies(
        &self,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Vec<u64>, PersistenceError>;
    fn dependency_reaches(
        &self,
        project: ProjectRecordId,
        from: u64,
        target: u64,
    ) -> Result<bool, PersistenceError>;
    fn add_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError>;
    fn remove_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError>;
    fn commit(self) -> Result<(), PersistenceError>;
}

pub struct WorkGraph<P> {
    persistence: P,
}

impl<P: Persistence> WorkGraph<P> {
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    pub fn search(&mut self, request: &SearchRequest) -> Result<SearchResult, WorkGraphError> {
        let transaction = self.persistence.begin(TransactionMode::Read)?;
        match request {
            SearchRequest::Status { project, status } => {
                let Some(id) = transaction.project_id(project)? else {
                    return Ok(SearchResult::Status(Vec::new()));
                };
                Ok(SearchResult::Status(
                    transaction.issues(project, id, *status)?,
                ))
            }
            SearchRequest::Issue { project, number } => {
                let id = transaction
                    .project_id(project)?
                    .ok_or(WorkGraphError::ProjectNotFound)?;
                let issue = transaction
                    .issue(project, id, *number)?
                    .ok_or(WorkGraphError::IssueNotFound)?;
                Ok(SearchResult::Issue(IssueDetail {
                    issue,
                    dependencies: transaction.dependencies(id, *number)?,
                    notes: transaction.notes(id, *number)?,
                }))
            }
            SearchRequest::Planning { project, planning } => {
                let Some(id) = transaction.project_id(project)? else {
                    return Ok(SearchResult::Planning(Vec::new()));
                };
                let mut issues = transaction.issues(project, id, None)?;
                issues.sort_by_key(|issue| (issue.priority, issue.created_at, issue.number));
                let mut result = Vec::new();
                for issue in issues.into_iter().filter(|issue| {
                    !matches!(issue.status, IssueStatus::Done | IssueStatus::Cancelled)
                }) {
                    let dependencies = transaction.dependencies(id, issue.number)?;
                    let mut unmet = false;
                    for dependency in dependencies {
                        let satisfied = transaction
                            .issue(project, id, dependency)?
                            .is_some_and(|item| item.status == IssueStatus::Done);
                        unmet |= !satisfied;
                    }
                    let blocked = issue.status == IssueStatus::Blocked || unmet;
                    let include = match planning {
                        PlanningView::Blocked => blocked,
                        PlanningView::Ready | PlanningView::Next => !blocked,
                    };
                    if include {
                        result.push(issue);
                        if *planning == PlanningView::Next {
                            break;
                        }
                    }
                }
                Ok(SearchResult::Planning(result))
            }
        }
    }

    pub fn edit(&mut self, request: &EditRequest) -> Result<EditResult, WorkGraphError> {
        validate_request(request)?;
        let fingerprint = serde_json::to_string(request)?;
        let mut transaction = self.persistence.begin(TransactionMode::Write)?;
        if let Some(receipt) = transaction.idempotency_receipt(&request.idempotency_key)? {
            if receipt.fingerprint != fingerprint {
                return Err(WorkGraphError::IdempotencyConflict);
            }
            return Ok(receipt.result);
        }
        let now = now_ms()?;
        let project = transaction.ensure_project(&request.project)?;
        let result = apply_edit(&mut transaction, request, project, now)?;
        transaction.record_idempotency(&request.idempotency_key, &fingerprint, &result, now)?;
        transaction.commit()?;
        Ok(result)
    }
}

fn apply_edit<T: WorkGraphTransaction>(
    transaction: &mut T,
    request: &EditRequest,
    project: ProjectRecordId,
    now: i64,
) -> Result<EditResult, WorkGraphError> {
    match &request.action {
        EditAction::Create {
            title,
            body,
            priority,
        } => {
            let number = transaction.next_issue_number(project)?;
            let issue = Issue {
                project: request.project.clone(),
                number,
                title: title.trim().to_owned(),
                body: body.clone(),
                status: IssueStatus::Open,
                priority: *priority,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            transaction.insert_issue(project, &issue)?;
            Ok(EditResult::Issue(issue))
        }
        EditAction::SetStatus {
            number,
            status,
            expected_version,
        } => {
            if !transaction.set_status(project, *number, *status, *expected_version, now)? {
                return Err(WorkGraphError::VersionConflict);
            }
            Ok(EditResult::Issue(required_issue(
                transaction,
                &request.project,
                project,
                *number,
            )?))
        }
        EditAction::AddNote {
            number,
            body,
            expected_version,
        } => {
            if !transaction.bump_version(project, *number, *expected_version, now)? {
                return Err(WorkGraphError::VersionConflict);
            }
            Ok(EditResult::Note(transaction.insert_note(
                project,
                *number,
                body.trim(),
                now,
            )?))
        }
        EditAction::AddDependency {
            number,
            depends_on,
            expected_version,
        } => {
            required_issue(transaction, &request.project, project, *depends_on)?;
            if number == depends_on
                || transaction.dependency_reaches(project, *depends_on, *number)?
            {
                return Err(WorkGraphError::DependencyCycle);
            }
            if !transaction.bump_version(project, *number, *expected_version, now)? {
                return Err(WorkGraphError::VersionConflict);
            }
            transaction.add_dependency(project, *number, *depends_on)?;
            Ok(EditResult::Issue(required_issue(
                transaction,
                &request.project,
                project,
                *number,
            )?))
        }
        EditAction::RemoveDependency {
            number,
            depends_on,
            expected_version,
        } => {
            if !transaction.bump_version(project, *number, *expected_version, now)? {
                return Err(WorkGraphError::VersionConflict);
            }
            transaction.remove_dependency(project, *number, *depends_on)?;
            Ok(EditResult::Issue(required_issue(
                transaction,
                &request.project,
                project,
                *number,
            )?))
        }
    }
}

fn required_issue<T: WorkGraphTransaction>(
    transaction: &T,
    path: &str,
    project: ProjectRecordId,
    number: u64,
) -> Result<Issue, WorkGraphError> {
    transaction
        .issue(path, project, number)?
        .ok_or(WorkGraphError::IssueNotFound)
}

fn validate_request(request: &EditRequest) -> Result<(), WorkGraphError> {
    if request.project.is_empty()
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 256
    {
        return Err(WorkGraphError::InvalidInput(
            "project and idempotency key are required",
        ));
    }
    match &request.action {
        EditAction::Create { title, body, .. }
            if title.trim().is_empty() || title.len() > 512 || body.len() > 1_000_000 =>
        {
            Err(WorkGraphError::InvalidInput(
                "issue title or body is invalid",
            ))
        }
        EditAction::AddNote { body, .. } if body.trim().is_empty() || body.len() > 100_000 => {
            Err(WorkGraphError::InvalidInput("note body is invalid"))
        }
        _ => Ok(()),
    }
}

fn now_ms() -> Result<i64, WorkGraphError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WorkGraphError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| WorkGraphError::Clock)
}
