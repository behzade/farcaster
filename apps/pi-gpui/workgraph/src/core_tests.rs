use std::collections::HashMap;

use crate::{
    contract::{
        CompletionRequirement, EditAction, EditRequest, EditResult, Evidence, EvidenceKind,
        IdempotencyReceipt, Outcome, SearchRequest, SearchResult, StoredProject,
    },
    core::{
        Persistence, PersistenceError, TransactionMode, WorkGraph, WorkGraphError,
        WorkGraphTransaction,
    },
};

#[derive(Default)]
struct MemoryPersistence {
    projects: HashMap<String, StoredProject>,
    receipts: HashMap<String, IdempotencyReceipt>,
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

    fn project(&self, project: &str) -> Result<Option<StoredProject>, PersistenceError> {
        Ok(self.state.projects.get(project).cloned())
    }

    fn save_project(
        &mut self,
        project: &str,
        value: &StoredProject,
        _updated_at: i64,
    ) -> Result<(), PersistenceError> {
        self.state.projects.insert(project.to_owned(), value.clone());
        Ok(())
    }

    fn commit(self) -> Result<(), PersistenceError> {
        Ok(())
    }
}

fn edit(graph: &mut WorkGraph<MemoryPersistence>, key: &str, action: EditAction) -> EditResult {
    graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: key.into(),
            action,
        })
        .expect("edit")
}

fn create_plan(graph: &mut WorkGraph<MemoryPersistence>) -> (u64, u64, u64) {
    let EditResult::Plan(snapshot) = edit(
        graph,
        "plan",
        EditAction::CreatePlan {
            title: "VCS integration".into(),
            root_title: "Current product".into(),
            files: vec!["apps/pi-gpui".into()],
            completion: CompletionRequirement::RevisionOrObservation,
        },
    ) else {
        panic!("plan result");
    };
    (
        snapshot.plan.number,
        snapshot.plan.root_node,
        snapshot.walk.expect("default walk").number,
    )
}

fn add_node(
    graph: &mut WorkGraph<MemoryPersistence>,
    key: &str,
    plan: u64,
    title: &str,
    after: Option<u64>,
) -> u64 {
    let EditResult::Node(node) = edit(
        graph,
        key,
        EditAction::AddNode {
            plan,
            title: title.into(),
            files: Vec::new(),
            completion: CompletionRequirement::RevisionOrObservation,
            after,
        },
    ) else {
        panic!("node result");
    };
    node.number
}

fn outcome(kind: EvidenceKind, reference: &str) -> Outcome {
    Outcome {
        note: "Verified state".into(),
        evidence: Evidence {
            kind,
            reference: reference.into(),
        },
    }
}

fn advance(
    graph: &mut WorkGraph<MemoryPersistence>,
    key: &str,
    walk: u64,
    number: u64,
    next: Option<u64>,
    version: u64,
) -> u64 {
    let EditResult::Step(step) = edit(
        graph,
        key,
        EditAction::Advance {
            walk,
            number,
            next,
            outcome: outcome(EvidenceKind::Revision, "git:abc123"),
            expected_version: Some(version),
        },
    ) else {
        panic!("step result");
    };
    step.id
}

#[test]
fn plan_walk_advances_only_across_direct_edges_and_persists_cursor() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, walk) = create_plan(&mut graph);
    let git = add_node(&mut graph, "git", plan, "Git backend", Some(root));
    let final_node = add_node(&mut graph, "final", plan, "Both backends", Some(git));

    let invalid = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "skip".into(),
        action: EditAction::Advance {
            walk,
            number: root,
            next: Some(final_node),
            outcome: outcome(EvidenceKind::Revision, "git:abc123"),
            expected_version: Some(1),
        },
    });
    assert!(matches!(invalid, Err(WorkGraphError::InvalidSuccessor)));

    advance(&mut graph, "advance-root", walk, root, None, 1);
    let SearchResult::Plan(snapshot) = graph
        .search(&SearchRequest::Plan {
            project: "/project".into(),
            plan,
            walk: Some(walk),
        })
        .expect("snapshot")
    else {
        panic!("plan search");
    };
    assert_eq!(snapshot.walk.expect("walk").current_node, Some(git));
    assert_eq!(snapshot.steps.len(), 1);
}

#[test]
fn branching_requires_a_successor_and_rewind_preserves_abandoned_history() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, walk) = create_plan(&mut graph);
    let git = add_node(&mut graph, "git", plan, "Git first", Some(root));
    let jj = add_node(&mut graph, "jj", plan, "jj first", Some(root));

    let ambiguous = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "ambiguous".into(),
        action: EditAction::Advance {
            walk,
            number: root,
            next: None,
            outcome: outcome(EvidenceKind::Revision, "git:abc123"),
            expected_version: Some(1),
        },
    });
    assert!(matches!(ambiguous, Err(WorkGraphError::AmbiguousSuccessor)));

    let root_step = advance(&mut graph, "choose-git", walk, root, Some(git), 1);
    advance(&mut graph, "finish-git", walk, git, None, 2);
    let EditResult::Walk(rewound) = edit(
        &mut graph,
        "rewind",
        EditAction::Rewind {
            walk,
            number: root,
            expected_version: Some(3),
        },
    ) else {
        panic!("walk result");
    };
    assert_eq!(rewound.current_node, Some(root));
    assert_eq!(rewound.head_step, None);
    advance(&mut graph, "choose-jj", walk, root, Some(jj), 4);

    let SearchResult::Plan(snapshot) = graph
        .search(&SearchRequest::Plan {
            project: "/project".into(),
            plan,
            walk: Some(walk),
        })
        .expect("snapshot")
    else {
        panic!("plan search");
    };
    assert_eq!(snapshot.steps.len(), 3);
    assert!(snapshot.steps.iter().any(|step| step.id == root_step));
    assert_eq!(snapshot.walk.expect("walk").current_node, Some(jj));
}

#[test]
fn reusable_plan_walks_advance_independently() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, first_walk) = create_plan(&mut graph);
    let next = add_node(&mut graph, "next", plan, "Integrated", Some(root));
    let EditResult::Walk(second_walk) = edit(
        &mut graph,
        "second-walk",
        EditAction::CreateWalk { plan },
    ) else {
        panic!("walk result");
    };

    advance(&mut graph, "first-advance", first_walk, root, None, 1);
    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project search");
    };
    assert_eq!(
        project
            .walks
            .iter()
            .find(|walk| walk.number == first_walk)
            .and_then(|walk| walk.current_node),
        Some(next)
    );
    assert_eq!(
        project
            .walks
            .iter()
            .find(|walk| walk.number == second_walk.number)
            .and_then(|walk| walk.current_node),
        Some(root)
    );
}

#[test]
fn observation_is_valid_when_a_code_state_already_exists() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (_plan, root, walk) = create_plan(&mut graph);
    let EditResult::Step(step) = edit(
        &mut graph,
        "observed",
        EditAction::Advance {
            walk,
            number: root,
            next: None,
            outcome: outcome(
                EvidenceKind::Observation,
                "Existing implementation at git:abc123; focused test passed",
            ),
            expected_version: Some(1),
        },
    ) else {
        panic!("step result");
    };
    assert_eq!(step.outcome.evidence.kind, EvidenceKind::Observation);
}

#[test]
fn core_rejects_cycles_stale_walks_and_idempotency_conflicts() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, walk) = create_plan(&mut graph);
    let next = add_node(&mut graph, "next", plan, "Next", Some(root));
    let cycle = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "cycle".into(),
        action: EditAction::AddEdge {
            plan,
            from: next,
            to: root,
        },
    });
    assert!(matches!(cycle, Err(WorkGraphError::DependencyCycle)));

    advance(&mut graph, "advance", walk, root, None, 1);
    let stale = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "stale".into(),
        action: EditAction::Advance {
            walk,
            number: next,
            next: None,
            outcome: outcome(EvidenceKind::Revision, "git:def456"),
            expected_version: Some(1),
        },
    });
    assert!(matches!(stale, Err(WorkGraphError::VersionConflict)));

    let conflict = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "plan".into(),
        action: EditAction::CreateWalk { plan },
    });
    assert!(matches!(conflict, Err(WorkGraphError::IdempotencyConflict)));
}
