use workgraph::{
    CompletionRequirement, Edge, Evidence, EvidenceKind, Node, Outcome, Plan, PlanSnapshot, Walk,
    WalkStep,
};

use super::*;

fn snapshot() -> PlanSnapshot {
    let node = |number: u64, title: &str| Node {
        plan_number: 1,
        number,
        title: title.into(),
        acceptance: String::new(),
        files: Vec::new(),
        completion: CompletionRequirement::RevisionOrObservation,
        version: 1,
        created_at: 0,
        updated_at: 0,
    };
    PlanSnapshot {
        plan: Plan {
            project: "/project".into(),
            number: 1,
            title: "VCS integration".into(),
            root_node: 1,
            version: 1,
            created_at: 0,
            updated_at: 0,
        },
        nodes: vec![
            node(1, "Current product"),
            node(2, "Git"),
            node(3, "jj"),
            node(4, "Both"),
        ],
        edges: vec![
            Edge {
                plan_number: 1,
                from: 1,
                to: 2,
            },
            Edge {
                plan_number: 1,
                from: 1,
                to: 3,
            },
            Edge {
                plan_number: 1,
                from: 2,
                to: 4,
            },
            Edge {
                plan_number: 1,
                from: 3,
                to: 4,
            },
        ],
        walk: Some(Walk {
            plan_number: 1,
            number: 1,
            current_node: Some(2),
            head_step: Some(1),
            version: 2,
            created_at: 0,
            updated_at: 0,
        }),
        steps: vec![WalkStep {
            id: 1,
            walk_number: 1,
            node_number: 1,
            parent_step: None,
            outcome: Outcome {
                note: "Baseline recorded".into(),
                evidence: Evidence {
                    kind: EvidenceKind::Revision,
                    reference: "git:abc".into(),
                },
            },
            completed_at: 0,
        }],
        sessions: Vec::new(),
    }
}

#[test]
fn global_completion_is_visible_without_a_step_on_the_selected_walk() {
    let snapshot = snapshot();
    let graph = workgraph::ProjectGraph {
        tasks: vec![workgraph::TaskState {
            plan_number: 1,
            task: 2,
            owner: None,
            completion: Some(workgraph::TaskCompletion {
                session_id: "another-session".into(),
                outcome: snapshot.steps[0].outcome.clone(),
                completed_at: 100,
            }),
        }],
        ..workgraph::ProjectGraph::default()
    };
    let rows = plan_rows(&snapshot, &graph, "Git");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].reached);
    assert!(!rows[0].current);
}

#[test]
fn projection_is_stable_and_marks_reached_and_current_nodes() {
    let rows = plan_rows(&snapshot(), &workgraph::ProjectGraph::default(), "");
    assert_eq!(
        rows.iter().map(|row| row.node.number).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(rows[0].reached);
    assert!(rows[1].current);
    assert!(rows.iter().all(|row| !row.detached));
}

#[test]
fn search_and_keyboard_navigation_use_visible_rows() {
    let rows = plan_rows(&snapshot(), &workgraph::ProjectGraph::default(), "jj");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].node.number, 3);
    assert_eq!(adjacent_node_number(&rows, None, 1), Some(3));
}

#[test]
fn plan_creation_requires_an_outcome_and_current_state() {
    assert!(!create_form_valid(false, "", "Current product"));
    assert!(!create_form_valid(false, "Git and jj", "  "));
    assert!(create_form_valid(false, "Git and jj", "Current product"));
    assert!(create_form_valid(true, "Add Git backend", ""));
}
