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
    advance(
        &mut graph,
        "complete-issue",
        walk.number,
        issue,
        None,
        walk.version,
    );
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
    let EditResult::Walk(second_walk) =
        edit(&mut graph, "second-walk", EditAction::CreateWalk { plan })
    else {
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

fn task_outcome(reference: &str) -> Outcome {
    outcome(EvidenceKind::Observation, reference)
}

#[test]
fn create_tasks_prepend_repositions_unstarted_walk_without_claiming() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, old_root, walk) = create_plan(&mut graph);
    let EditResult::Tasks(snapshot) = edit(
        &mut graph,
        "prepend-task",
        EditAction::CreateTasks {
            nodes: vec![NodeDraft {
                title: "New root".into(),
                acceptance: "Ready".into(),
            }],
            after: None,
            before: Some(old_root),
        },
    ) else {
        panic!("tasks result");
    };
    let new_root = snapshot.plan.root_node;

    assert_eq!(snapshot.plan.number, plan);
    assert_ne!(new_root, old_root);
    assert_eq!(
        snapshot
            .walk
            .as_ref()
            .filter(|candidate| candidate.number == walk)
            .and_then(|candidate| candidate.current_node),
        Some(new_root)
    );
    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project result");
    };
    assert!(
        project
            .task_state(new_root)
            .is_some_and(|state| state.owner.is_none())
    );
}

#[test]
fn task_claims_are_exclusive_idempotent_and_single_per_session() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let EditResult::Tasks(created) = edit(
        &mut graph,
        "create-tasks",
        EditAction::CreateTasks {
            nodes: vec![NodeDraft {
                title: "First".into(),
                acceptance: "Done".into(),
            }],
            after: None,
            before: None,
        },
    ) else {
        panic!("tasks result");
    };
    let first = created.plan.root_node;
    let EditResult::Task(claimed) = edit(
        &mut graph,
        "claim-first",
        EditAction::ClaimTask {
            task: first,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    ) else {
        panic!("task result");
    };
    let claimed_at = claimed.owner.expect("owner").claimed_at;
    let independent = add_node(
        &mut graph,
        "independent-task",
        created.plan.number,
        "Independent",
        None,
    );
    let second_for_same_session = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "second-for-same-session".into(),
        action: EditAction::ClaimTask {
            task: independent,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    });
    assert!(matches!(
        second_for_same_session,
        Err(WorkGraphError::ActiveTaskConflict)
    ));

    let EditResult::Task(reclaimed) = edit(
        &mut graph,
        "claim-first-again",
        EditAction::ClaimTask {
            task: first,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    ) else {
        panic!("task result");
    };
    assert_eq!(reclaimed.owner.expect("owner").claimed_at, claimed_at);

    let other = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "other-claim".into(),
        action: EditAction::ClaimTask {
            task: first,
            session_id: "session-2".into(),
            session_path: "/sessions/two.jsonl".into(),
        },
    });
    assert!(matches!(other, Err(WorkGraphError::TaskClaimed)));
}

#[test]
fn completion_is_global_unlocks_blockers_and_does_not_auto_advance() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let EditResult::Tasks(created) = edit(
        &mut graph,
        "create-chain",
        EditAction::CreateTasks {
            nodes: vec![
                NodeDraft {
                    title: "First".into(),
                    acceptance: "Done".into(),
                },
                NodeDraft {
                    title: "Second".into(),
                    acceptance: "Done".into(),
                },
            ],
            after: None,
            before: None,
        },
    ) else {
        panic!("tasks result");
    };
    let first = created.nodes[0].number;
    let second = created.nodes[1].number;
    let blocked = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "blocked".into(),
        action: EditAction::ClaimTask {
            task: second,
            session_id: "session-2".into(),
            session_path: "/sessions/two.jsonl".into(),
        },
    });
    assert!(matches!(blocked, Err(WorkGraphError::TaskBlocked)));

    edit(
        &mut graph,
        "claim-first",
        EditAction::ClaimTask {
            task: first,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );
    let EditResult::Task(completed) = edit(
        &mut graph,
        "complete-first",
        EditAction::CompleteTask {
            task: first,
            session_id: "session-1".into(),
            outcome: task_outcome("tests passed"),
        },
    ) else {
        panic!("task result");
    };
    assert!(completed.owner.is_none());
    assert_eq!(
        completed.completion.as_ref().map(|completion| completion
            .outcome
            .evidence
            .reference
            .as_str()),
        Some("tests passed")
    );

    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project result");
    };
    let walk = project
        .sessions
        .iter()
        .find(|link| link.session_id == "session-1")
        .and_then(|link| {
            project
                .walks
                .iter()
                .find(|walk| walk.number == link.walk_number)
        })
        .expect("claimed walk");
    assert_eq!(walk.current_node, None);

    edit(
        &mut graph,
        "claim-second",
        EditAction::ClaimTask {
            task: second,
            session_id: "session-2".into(),
            session_path: "/sessions/two.jsonl".into(),
        },
    );
}

#[test]
fn claim_does_not_reposition_a_walk_shared_with_another_session() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (_plan, root, shared_walk) = create_plan(&mut graph);
    for (key, session_id) in [("link-one", "session-1"), ("link-two", "session-2")] {
        edit(
            &mut graph,
            key,
            EditAction::LinkSession {
                walk: shared_walk,
                session_id: session_id.into(),
                session_path: format!("/sessions/{session_id}.jsonl"),
            },
        );
    }
    edit(
        &mut graph,
        "claim-shared",
        EditAction::ClaimTask {
            task: root,
            session_id: "session-1".into(),
            session_path: "/sessions/session-1.jsonl".into(),
        },
    );

    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project result");
    };
    let first = project
        .sessions
        .iter()
        .find(|link| link.session_id == "session-1")
        .expect("first link");
    let second = project
        .sessions
        .iter()
        .find(|link| link.session_id == "session-2")
        .expect("second link");
    assert_ne!(first.walk_number, second.walk_number);
    assert_eq!(second.walk_number, shared_walk);
}

#[test]
fn active_claim_prevents_relinking_or_unlinking_its_session() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, _) = create_plan(&mut graph);
    let EditResult::Walk(other_walk) =
        edit(&mut graph, "other-walk", EditAction::CreateWalk { plan })
    else {
        panic!("walk result");
    };
    edit(
        &mut graph,
        "claim-root",
        EditAction::ClaimTask {
            task: root,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );
    let relink = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "relink-active".into(),
        action: EditAction::LinkSession {
            walk: other_walk.number,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    });
    assert!(matches!(relink, Err(WorkGraphError::ActiveTaskConflict)));
    let unlink = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "unlink-active".into(),
        action: EditAction::UnlinkSession {
            session_id: "session-1".into(),
        },
    });
    assert!(matches!(unlink, Err(WorkGraphError::ActiveTaskConflict)));
}

#[test]
fn release_requires_owner_and_clears_the_claim_walk() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let EditResult::Tasks(created) = edit(
        &mut graph,
        "create-task",
        EditAction::CreateTasks {
            nodes: vec![NodeDraft {
                title: "Task".into(),
                acceptance: "Done".into(),
            }],
            after: None,
            before: None,
        },
    ) else {
        panic!("tasks result");
    };
    let task = created.plan.root_node;
    edit(
        &mut graph,
        "claim",
        EditAction::ClaimTask {
            task,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );
    let wrong_owner = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "wrong-release".into(),
        action: EditAction::ReleaseTask {
            task,
            session_id: "session-2".into(),
        },
    });
    assert!(matches!(wrong_owner, Err(WorkGraphError::NotTaskOwner)));
    let EditResult::Task(released) = edit(
        &mut graph,
        "release",
        EditAction::ReleaseTask {
            task,
            session_id: "session-1".into(),
        },
    ) else {
        panic!("task result");
    };
    assert!(released.owner.is_none());
}

#[test]
fn legacy_advance_projects_global_completion_for_persisted_task_state() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (_plan, root, walk) = create_plan(&mut graph);
    advance(&mut graph, "legacy-advance", walk, root, None, 1);

    let SearchResult::Project(project) = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project result");
    };
    assert!(
        project
            .task_state(root)
            .is_some_and(|state| state.completion.is_some())
    );
    assert!(
        project
            .tasks
            .iter()
            .find(|state| state.task == root)
            .is_some_and(|state| state.completion.is_some())
    );
}

#[test]
fn rewind_cannot_move_a_walk_positioned_by_an_active_claim() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, walk) = create_plan(&mut graph);
    let next = add_node(&mut graph, "next-for-claim", plan, "Next", Some(root));
    advance(&mut graph, "finish-root", walk, root, None, 1);
    edit(
        &mut graph,
        "link-before-claim",
        EditAction::LinkSession {
            walk,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );
    edit(
        &mut graph,
        "claim-next",
        EditAction::ClaimTask {
            task: next,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );

    let rewind = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "rewind-claimed-walk".into(),
        action: EditAction::Rewind {
            walk,
            number: root,
            expected_version: None,
        },
    });
    assert!(matches!(rewind, Err(WorkGraphError::TaskClaimed)));
}

#[test]
fn legacy_completion_cannot_bypass_an_exclusive_claim() {
    let mut graph = WorkGraph::new(MemoryPersistence::default());
    let (plan, root, walk) = create_plan(&mut graph);
    edit(
        &mut graph,
        "claim-root",
        EditAction::ClaimTask {
            task: root,
            session_id: "session-1".into(),
            session_path: "/sessions/one.jsonl".into(),
        },
    );
    let result = graph.edit(&EditRequest {
        project: "/project".into(),
        idempotency_key: "legacy-advance".into(),
        action: EditAction::Advance {
            walk,
            number: root,
            next: None,
            outcome: task_outcome("bypass"),
            expected_version: None,
        },
    });
    assert!(matches!(result, Err(WorkGraphError::TaskClaimed)));
    let _ = plan;
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
