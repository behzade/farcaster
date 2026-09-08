use super::*;
use crate::{
    CompletionRequirement, Edge, Evidence, EvidenceKind, Node, Outcome, Plan, TaskCompletion,
    TaskOwner, TaskState, WalkStep,
};

fn graph() -> ProjectGraph {
    ProjectGraph {
        plans: vec![Plan {
            project: "/project".into(),
            number: 1,
            title: "Shipping".into(),
            root_node: 1,
            version: 1,
            created_at: 1,
            updated_at: 1,
        }],
        nodes: (1..=3)
            .map(|number| Node {
                plan_number: 1,
                number,
                title: format!("Task {number}"),
                acceptance: String::new(),
                files: vec![],
                completion: CompletionRequirement::default(),
                version: 1,
                created_at: 1,
                updated_at: 1,
            })
            .collect(),
        edges: vec![
            Edge {
                plan_number: 1,
                from: 1,
                to: 2,
            },
            Edge {
                plan_number: 1,
                from: 2,
                to: 3,
            },
        ],
        ..ProjectGraph::default()
    }
}

fn outcome() -> Outcome {
    Outcome {
        note: "Verified".into(),
        evidence: Evidence {
            kind: EvidenceKind::Observation,
            reference: "check passed".into(),
        },
    }
}

fn complete(graph: &mut ProjectGraph, task: u64) {
    graph.tasks.push(TaskState {
        plan_number: 1,
        task,
        owner: None,
        completion: Some(TaskCompletion {
            session_id: "session".into(),
            outcome: outcome(),
            completed_at: 100 + task as i64,
        }),
    });
}

#[test]
fn dependency_chain_is_ready_until_claimed_and_done_only_when_all_tasks_finish() {
    let mut graph = graph();
    assert_eq!(graph.work_status(1), WorkStatus::Ready);
    assert_eq!(graph.work_status(2), WorkStatus::Blocked);
    assert_eq!(graph.plan_overviews()[0].status, WorkStatus::Ready);
    graph.tasks.push(TaskState {
        plan_number: 1,
        task: 1,
        completion: None,
        owner: Some(TaskOwner {
            session_id: "session".into(),
            session_path: "/session".into(),
            claimed_at: 50,
        }),
    });
    assert_eq!(graph.plan_overviews()[0].status, WorkStatus::Active);
    assert_eq!(graph.plan_overviews()[0].updated_at, 50);
    graph.tasks.clear();
    complete(&mut graph, 1);
    assert_eq!(graph.work_status(2), WorkStatus::Ready);
    assert_eq!(graph.work_status(3), WorkStatus::Blocked);
    assert_eq!(graph.plan_overviews()[0].status, WorkStatus::Active);
    complete(&mut graph, 2);
    complete(&mut graph, 3);
    let row = &graph.plan_overviews()[0];
    assert_eq!(
        (row.status, row.done, row.total, row.blocked),
        (WorkStatus::Done, 3, 3, 0)
    );
    assert_eq!(row.updated_at, 103);
}

#[test]
fn legacy_outcomes_count_even_with_uncompleted_task_state() {
    let mut graph = graph();
    graph.tasks.push(TaskState {
        plan_number: 1,
        task: 1,
        owner: None,
        completion: None,
    });
    graph.steps.push(WalkStep {
        id: 1,
        walk_number: 1,
        node_number: 1,
        parent_step: None,
        outcome: outcome(),
        completed_at: 90,
    });
    assert_eq!(graph.work_status(1), WorkStatus::Done);
    assert_eq!(graph.work_status(2), WorkStatus::Ready);
    assert_eq!(graph.plan_overviews()[0].done, 1);
    assert_eq!(graph.plan_overviews()[0].updated_at, 90);
}
