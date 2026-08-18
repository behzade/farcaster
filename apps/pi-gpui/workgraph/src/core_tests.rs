use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    contract::{
        EditAction, EditRequest, EditResult, IdempotencyReceipt, Issue, IssueStatus, Note,
        PlanningView, ProjectRecordId, SearchRequest, SearchResult,
    },
    core::{
        Persistence, PersistenceError, TransactionMode, WorkGraph, WorkGraphError,
        WorkGraphTransaction,
    },
};

#[derive(Default)]
struct MemoryPersistence {
    project_ids: HashMap<String, ProjectRecordId>,
    next_project_id: i64,
    next_numbers: HashMap<ProjectRecordId, u64>,
    issues: BTreeMap<(ProjectRecordId, u64), Issue>,
    notes: BTreeMap<(ProjectRecordId, u64), Vec<Note>>,
    dependencies: BTreeSet<(ProjectRecordId, u64, u64)>,
    receipts: HashMap<String, IdempotencyReceipt>,
    next_note_id: i64,
}

struct MemoryTransaction<'a> {
    state: &'a mut MemoryPersistence,
}

impl Persistence for MemoryPersistence {
    type Transaction<'a> = MemoryTransaction<'a>;

    fn begin(&mut self, _mode: TransactionMode) -> Result<Self::Transaction<'_>, PersistenceError> {
        Ok(MemoryTransaction { state: self })
    }
}

impl WorkGraphTransaction for MemoryTransaction<'_> {
    fn idempotency_receipt(
        &self,
        key: &str,
    ) -> Result<Option<IdempotencyReceipt>, PersistenceError> {
        Ok(self.state.receipts.get(key).cloned())
    }

    fn record_idempotency(
        &mut self,
        key: &str,
        fingerprint: &str,
        result: &EditResult,
        _created_at: i64,
    ) -> Result<(), PersistenceError> {
        self.state.receipts.insert(
            key.to_owned(),
            IdempotencyReceipt {
                fingerprint: fingerprint.to_owned(),
                result: result.clone(),
            },
        );
        Ok(())
    }

    fn ensure_project(&mut self, project: &str) -> Result<ProjectRecordId, PersistenceError> {
        if let Some(id) = self.state.project_ids.get(project) {
            return Ok(*id);
        }
        self.state.next_project_id += 1;
        let id = ProjectRecordId::from_storage(self.state.next_project_id);
        self.state.project_ids.insert(project.to_owned(), id);
        self.state.next_numbers.insert(id, 1);
        Ok(id)
    }

    fn project_id(&self, project: &str) -> Result<Option<ProjectRecordId>, PersistenceError> {
        Ok(self.state.project_ids.get(project).copied())
    }

    fn next_issue_number(&mut self, project: ProjectRecordId) -> Result<u64, PersistenceError> {
        let next = self
            .state
            .next_numbers
            .get_mut(&project)
            .ok_or_else(|| PersistenceError::new("project missing"))?;
        let number = *next;
        *next += 1;
        Ok(number)
    }

    fn insert_issue(
        &mut self,
        project: ProjectRecordId,
        issue: &Issue,
    ) -> Result<(), PersistenceError> {
        self.state
            .issues
            .insert((project, issue.number), issue.clone());
        Ok(())
    }

    fn issue(
        &self,
        _project_path: &str,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Option<Issue>, PersistenceError> {
        Ok(self.state.issues.get(&(project, number)).cloned())
    }

    fn issues(
        &self,
        _project_path: &str,
        project: ProjectRecordId,
        status: Option<IssueStatus>,
    ) -> Result<Vec<Issue>, PersistenceError> {
        let mut issues = self
            .state
            .issues
            .iter()
            .filter(|((id, _), issue)| *id == project && status.is_none_or(|s| issue.status == s))
            .map(|(_, issue)| issue.clone())
            .collect::<Vec<_>>();
        issues.sort_by_key(|issue| (issue.priority, issue.created_at, issue.number));
        Ok(issues)
    }

    fn set_status(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        status: IssueStatus,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError> {
        let Some(issue) = self.state.issues.get_mut(&(project, number)) else {
            return Ok(false);
        };
        if expected_version.is_some_and(|version| version != issue.version) {
            return Ok(false);
        }
        issue.status = status;
        issue.version += 1;
        issue.updated_at = updated_at;
        Ok(true)
    }

    fn bump_version(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        expected_version: Option<u64>,
        updated_at: i64,
    ) -> Result<bool, PersistenceError> {
        let Some(issue) = self.state.issues.get_mut(&(project, number)) else {
            return Ok(false);
        };
        if expected_version.is_some_and(|version| version != issue.version) {
            return Ok(false);
        }
        issue.version += 1;
        issue.updated_at = updated_at;
        Ok(true)
    }

    fn insert_note(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        body: &str,
        created_at: i64,
    ) -> Result<Note, PersistenceError> {
        self.state.next_note_id += 1;
        let note = Note {
            id: self.state.next_note_id,
            issue_number: number,
            body: body.to_owned(),
            created_at,
        };
        self.state
            .notes
            .entry((project, number))
            .or_default()
            .push(note.clone());
        Ok(note)
    }

    fn notes(&self, project: ProjectRecordId, number: u64) -> Result<Vec<Note>, PersistenceError> {
        Ok(self
            .state
            .notes
            .get(&(project, number))
            .cloned()
            .unwrap_or_default())
    }

    fn dependencies(
        &self,
        project: ProjectRecordId,
        number: u64,
    ) -> Result<Vec<u64>, PersistenceError> {
        Ok(self
            .state
            .dependencies
            .iter()
            .filter(|(id, issue, _)| *id == project && *issue == number)
            .map(|(_, _, dependency)| *dependency)
            .collect())
    }

    fn dependency_reaches(
        &self,
        project: ProjectRecordId,
        from: u64,
        target: u64,
    ) -> Result<bool, PersistenceError> {
        let mut pending = vec![from];
        let mut seen = BTreeSet::new();
        while let Some(number) = pending.pop() {
            if !seen.insert(number) {
                continue;
            }
            for (_, _, dependency) in self
                .state
                .dependencies
                .iter()
                .filter(|(id, issue, _)| *id == project && *issue == number)
            {
                if *dependency == target {
                    return Ok(true);
                }
                pending.push(*dependency);
            }
        }
        Ok(false)
    }

    fn add_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError> {
        self.state
            .dependencies
            .insert((project, number, depends_on));
        Ok(())
    }

    fn remove_dependency(
        &mut self,
        project: ProjectRecordId,
        number: u64,
        depends_on: u64,
    ) -> Result<(), PersistenceError> {
        self.state
            .dependencies
            .remove(&(project, number, depends_on));
        Ok(())
    }

    fn commit(self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

fn create(graph: &mut WorkGraph<MemoryPersistence>, key: &str, title: &str) -> Issue {
    create_with_priority(graph, key, title, 0)
}

fn create_with_priority(
    graph: &mut WorkGraph<MemoryPersistence>,
    key: &str,
    title: &str,
    priority: u64,
) -> Issue {
    let EditResult::Issue(issue) = graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: key.into(),
            action: EditAction::Create {
                title: title.into(),
                body: String::new(),
                priority,
            },
        })
        .expect("create issue")
    else {
        panic!("issue result");
    };
    issue
}

#[test]
fn core_runs_against_a_non_sqlite_persistence() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let first = create(&mut graph, "first", "First");
    let second = create(&mut graph, "second", "Second");
    graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "dependency".into(),
            action: EditAction::AddDependency {
                number: second.number,
                depends_on: first.number,
                expected_version: Some(second.version),
            },
        })
        .expect("add dependency");
    let blocked = graph
        .search(&SearchRequest::Planning {
            project: "/project".into(),
            planning: PlanningView::Blocked,
        })
        .expect("blocked planning");
    assert!(
        matches!(blocked, SearchResult::Planning(items) if items.len() == 1 && items[0].number == second.number)
    );
}

#[test]
fn next_uses_canonical_lowest_priority_then_creation_order() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let later = create_with_priority(&mut graph, "later", "Later", 2);
    let next = create_with_priority(&mut graph, "next", "Next", 0);
    let middle = create_with_priority(&mut graph, "middle", "Middle", 1);

    let ready = graph
        .search(&SearchRequest::Planning {
            project: "/project".into(),
            planning: PlanningView::Ready,
        })
        .expect("ready planning");
    assert!(matches!(
        ready,
        SearchResult::Planning(items)
            if items.iter().map(|issue| issue.number).collect::<Vec<_>>()
                == vec![next.number, middle.number, later.number]
    ));
    let first = graph
        .search(&SearchRequest::Planning {
            project: "/project".into(),
            planning: PlanningView::Next,
        })
        .expect("next planning");
    assert!(matches!(first, SearchResult::Planning(items) if items[0].number == next.number));
}

#[test]
fn core_owns_idempotency_cycles_and_version_invariants() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let first = create(&mut graph, "first", "First");
    assert_eq!(first, create(&mut graph, "first", "First"));
    let conflict = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "first".into(),
        action: EditAction::Create {
            title: "Changed".into(),
            body: String::new(),
            priority: 0,
        },
    });
    assert!(matches!(conflict, Err(WorkGraphError::IdempotencyConflict)));

    let second = create(&mut graph, "second", "Second");
    graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "one-two".into(),
            action: EditAction::AddDependency {
                number: first.number,
                depends_on: second.number,
                expected_version: Some(first.version),
            },
        })
        .expect("first dependency");
    let cycle = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "two-one".into(),
        action: EditAction::AddDependency {
            number: second.number,
            depends_on: first.number,
            expected_version: Some(second.version),
        },
    });
    assert!(matches!(cycle, Err(WorkGraphError::DependencyCycle)));
}
