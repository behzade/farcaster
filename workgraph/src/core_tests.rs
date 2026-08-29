use std::collections::HashMap;

use crate::{
    contract::{
        CompletionRequirement, EditAction, EditRequest, EditResult, Evidence, EvidenceKind,
        IdempotencyReceipt, NodeDraft, Outcome, SearchRequest, SearchResult, StoredProject,
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
            files: vec!["apps/farcaster".into()],
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

fn patch(
    graph: &mut WorkGraph<MemoryPersistence>,
    key: &str,
    nodes: &[(&str, &str)],
    after: Option<u64>,
    before: Option<u64>,
) -> crate::contract::PlanSnapshot {
    let EditResult::Plan(snapshot) = edit(
        graph,
        key,
        EditAction::Patch {
            nodes: nodes
                .iter()
                .map(|(title, acceptance)| NodeDraft {
                    title: (*title).into(),
                    acceptance: (*acceptance).into(),
                })
                .collect(),
            after,
            before,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    ) else {
        panic!("patch result");
    };
    snapshot
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
fn patch_without_attachments_creates_and_attaches_an_ordered_graph() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let snapshot = patch(
        &mut graph,
        "patch-create",
        &[
            ("Issue", "Problem is understood"),
            ("Outcome", "Behavior is fixed"),
        ],
        None,
        None,
    );

    assert_eq!(snapshot.plan.root_node, snapshot.nodes[0].number);
    assert_eq!(snapshot.nodes[0].acceptance, "Problem is understood");
    assert_eq!(
        snapshot.walk.expect("walk").current_node,
        Some(snapshot.nodes[0].number)
    );
    assert_eq!(snapshot.edges.len(), 1);
    assert_eq!(snapshot.edges[0].from, snapshot.nodes[0].number);
    assert_eq!(snapshot.edges[0].to, snapshot.nodes[1].number);
    assert_eq!(snapshot.sessions[0].session_id, "session-1");
}

#[test]
fn patch_prepends_and_repositions_unstarted_walks() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let created = patch(
        &mut graph,
        "patch-create",
        &[("Issue", "Known"), ("Outcome", "Fixed")],
        None,
        None,
    );
    let old_root = created.plan.root_node;
    let snapshot = patch(
        &mut graph,
        "patch-prepend",
        &[("Earlier issue", "Reproduced")],
        None,
        Some(old_root),
    );
    let new_root = snapshot.plan.root_node;

    assert_ne!(new_root, old_root);
    assert_eq!(snapshot.walk.expect("walk").current_node, Some(new_root));
    assert!(
        snapshot
            .edges
            .iter()
            .any(|edge| edge.from == new_root && edge.to == old_root)
    );
}

#[test]
fn patch_after_a_completed_leaf_reactivates_the_walk() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let created = patch(
        &mut graph,
        "patch-create",
        &[("Issue", "Known"), ("Outcome", "Fixed")],
        None,
        None,
    );
    let issue = created.nodes[0].number;
    let outcome_node = created.nodes[1].number;
    let walk = created.walk.expect("walk");
    advance(&mut graph, "complete-issue", walk.number, issue, None, walk.version);
    advance(
        &mut graph,
        "complete-outcome",
        walk.number,
        outcome_node,
        None,
        walk.version + 1,
    );

    let snapshot = patch(
        &mut graph,
        "patch-after-leaf",
        &[("Follow-up", "Verified")],
        Some(outcome_node),
        None,
    );
    let follow_up = snapshot
        .nodes
        .iter()
        .find(|node| node.title == "Follow-up")
        .expect("follow-up")
        .number;

    assert_eq!(snapshot.walk.expect("walk").current_node, Some(follow_up));
}

#[test]
fn patch_cannot_move_the_session_to_another_graph() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let attached = patch(
        &mut graph,
        "patch-create",
        &[("Issue", "Known"), ("Outcome", "Fixed")],
        None,
        None,
    );
    let (other_plan, other_root, _) = create_plan(&mut graph);
    let result = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "cross-graph-patch".into(),
        action: EditAction::Patch {
            nodes: vec![NodeDraft {
                title: "Wrong graph".into(),
                acceptance: "Never attached".into(),
            }],
            after: Some(other_root),
            before: None,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    });

    assert!(matches!(result, Err(WorkGraphError::InvalidInput(_))));
    let SearchResult::Session(link) = graph
        .search(&SearchRequest::Session {
            project: "/project".into(),
            session_id: "session-1".into(),
        })
        .expect("session search")
    else {
        panic!("session result");
    };
    assert_eq!(link.expect("attached session").plan_number, attached.plan.number);
    assert_ne!(attached.plan.number, other_plan);
}

#[test]
fn patch_between_completed_predecessor_and_active_node_moves_cursor() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let created = patch(
        &mut graph,
        "patch-create",
        &[("Issue", "Known"), ("Outcome", "Fixed")],
        None,
        None,
    );
    let issue = created.nodes[0].number;
    let outcome_node = created.nodes[1].number;
    let walk = created.walk.expect("walk");
    advance(
        &mut graph,
        "complete-issue",
        walk.number,
        issue,
        None,
        walk.version,
    );

    let snapshot = patch(
        &mut graph,
        "patch-middle",
        &[("Implementation", "Regression test passes")],
        Some(issue),
        Some(outcome_node),
    );
    let inserted = snapshot
        .nodes
        .iter()
        .find(|node| node.title == "Implementation")
        .expect("inserted")
        .number;

    assert_eq!(snapshot.walk.expect("walk").current_node, Some(inserted));
    assert!(
        !snapshot
            .edges
            .iter()
            .any(|edge| edge.from == issue && edge.to == outcome_node)
    );
    assert!(
        snapshot
            .edges
            .iter()
            .any(|edge| edge.from == issue && edge.to == inserted)
    );
    assert!(
        snapshot
            .edges
            .iter()
            .any(|edge| edge.from == inserted && edge.to == outcome_node)
    );
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
fn session_completion_adapts_generic_evidence_to_legacy_requirements() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let EditResult::Plan(snapshot) = edit(
        &mut graph,
        "file-plan",
        EditAction::CreatePlan {
            title: "Legacy file plan".into(),
            root_title: "Write artifact".into(),
            files: Vec::new(),
            completion: CompletionRequirement::File,
        },
    ) else {
        panic!("plan result");
    };
    let walk = snapshot.walk.expect("walk").number;
    edit(
        &mut graph,
        "link-file-plan",
        EditAction::LinkSession {
            walk,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );

    let EditResult::Plan(snapshot) = edit(
        &mut graph,
        "complete-file-node",
        EditAction::Complete {
            session_id: "session-1".into(),
            next: None,
            outcome: outcome(EvidenceKind::Observation, "artifact.txt"),
        },
    ) else {
        panic!("complete result");
    };

    assert_eq!(snapshot.steps[0].outcome.evidence.kind, EvidenceKind::File);
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
