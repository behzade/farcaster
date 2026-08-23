//! Pure plan-list projection independent of GPUI and SQLite.

use std::collections::{HashMap, HashSet, VecDeque};

use workgraph::contract::{PlanSnapshot, WalkStep};

use super::contract::PlanRow;

pub(super) fn plan_rows(snapshot: &PlanSnapshot, search: &str) -> Vec<PlanRow> {
    let reached = active_steps(snapshot)
        .into_iter()
        .map(|step| step.node_number)
        .collect::<HashSet<_>>();
    let current = snapshot.walk.as_ref().and_then(|walk| walk.current_node);
    let reachable = reachable_nodes(snapshot);
    let depths = node_depths(snapshot);
    topological_numbers(snapshot)
        .into_iter()
        .filter_map(|number| {
            let node = snapshot.nodes.iter().find(|node| node.number == number)?;
            node_matches(node, search).then(|| PlanRow {
                node: node.clone(),
                depth: depths.get(&number).copied().unwrap_or_default(),
                reached: reached.contains(&number),
                current: current == Some(number),
                detached: !reachable.contains(&number),
            })
        })
        .collect()
}

pub(super) fn active_steps(snapshot: &PlanSnapshot) -> Vec<&WalkStep> {
    let Some(walk) = &snapshot.walk else {
        return Vec::new();
    };
    let mut reversed = Vec::new();
    let mut current = walk.head_step;
    while let Some(id) = current {
        let Some(step) = snapshot.steps.iter().find(|step| step.id == id) else {
            break;
        };
        reversed.push(step);
        current = step.parent_step;
    }
    reversed.reverse();
    reversed
}

pub(super) fn active_outcome(snapshot: &PlanSnapshot, number: u64) -> Option<&WalkStep> {
    active_steps(snapshot)
        .into_iter()
        .find(|step| step.node_number == number)
}

pub(super) fn create_form_valid(has_plan: bool, title: &str, detail: &str) -> bool {
    !title.trim().is_empty() && (has_plan || !detail.trim().is_empty())
}

pub(super) fn adjacent_node_number(
    rows: &[PlanRow],
    selected: Option<u64>,
    delta: isize,
) -> Option<u64> {
    if rows.is_empty() {
        return None;
    }
    let current = selected
        .and_then(|number| rows.iter().position(|row| row.node.number == number))
        .unwrap_or(if delta < 0 { 0 } else { rows.len() - 1 });
    let next = (current as isize + delta).rem_euclid(rows.len() as isize) as usize;
    rows.get(next).map(|row| row.node.number)
}

fn node_matches(node: &workgraph::contract::Node, search: &str) -> bool {
    let search = search.trim().to_lowercase();
    if search.is_empty() {
        return true;
    }
    let value = format!(
        "#{} {} {} {}",
        node.number,
        node.title,
        node.acceptance,
        node.files.join(" ")
    );
    value.to_lowercase().contains(&search)
}

fn topological_numbers(snapshot: &PlanSnapshot) -> Vec<u64> {
    let mut indegree = snapshot
        .nodes
        .iter()
        .map(|node| (node.number, 0_usize))
        .collect::<HashMap<_, _>>();
    for edge in &snapshot.edges {
        if let Some(value) = indegree.get_mut(&edge.to) {
            *value = value.saturating_add(1);
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(number, degree)| (*degree == 0).then_some(*number))
        .collect::<Vec<_>>();
    ready.sort_unstable();
    let mut ready = VecDeque::from(ready);
    let mut result = Vec::new();
    while let Some(number) = ready.pop_front() {
        result.push(number);
        let mut successors = snapshot
            .edges
            .iter()
            .filter(|edge| edge.from == number)
            .map(|edge| edge.to)
            .collect::<Vec<_>>();
        successors.sort_unstable();
        for successor in successors {
            let Some(degree) = indegree.get_mut(&successor) else {
                continue;
            };
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                let insertion = ready
                    .iter()
                    .position(|queued| *queued > successor)
                    .unwrap_or(ready.len());
                ready.insert(insertion, successor);
            }
        }
    }
    for node in &snapshot.nodes {
        if !result.contains(&node.number) {
            result.push(node.number);
        }
    }
    result
}

fn reachable_nodes(snapshot: &PlanSnapshot) -> HashSet<u64> {
    let mut reached = HashSet::new();
    let mut pending = vec![snapshot.plan.root_node];
    while let Some(number) = pending.pop() {
        if !reached.insert(number) {
            continue;
        }
        pending.extend(
            snapshot
                .edges
                .iter()
                .filter(|edge| edge.from == number)
                .map(|edge| edge.to),
        );
    }
    reached
}

fn node_depths(snapshot: &PlanSnapshot) -> HashMap<u64, usize> {
    let order = topological_numbers(snapshot);
    let mut depths = HashMap::from([(snapshot.plan.root_node, 0_usize)]);
    for number in order {
        let depth = depths.get(&number).copied().unwrap_or_default();
        for successor in snapshot
            .edges
            .iter()
            .filter(|edge| edge.from == number)
            .map(|edge| edge.to)
        {
            depths
                .entry(successor)
                .and_modify(|value| *value = (*value).max(depth.saturating_add(1)))
                .or_insert(depth.saturating_add(1));
        }
    }
    depths
}

#[cfg(test)]
mod tests {
    use workgraph::contract::{
        CompletionRequirement, Edge, Evidence, EvidenceKind, Node, Outcome, Plan, PlanSnapshot,
        Walk, WalkStep,
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
    fn projection_is_stable_and_marks_reached_current_and_depth() {
        let rows = plan_rows(&snapshot(), "");
        assert_eq!(
            rows.iter().map(|row| row.node.number).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert!(rows[0].reached);
        assert!(rows[1].current);
        assert_eq!(rows[3].depth, 2);
        assert!(rows.iter().all(|row| !row.detached));
    }

    #[test]
    fn active_chain_ignores_abandoned_steps() {
        let mut snapshot = snapshot();
        snapshot.steps.push(WalkStep {
            id: 2,
            walk_number: 1,
            node_number: 3,
            parent_step: Some(1),
            outcome: snapshot.steps[0].outcome.clone(),
            completed_at: 1,
        });
        assert_eq!(active_steps(&snapshot).len(), 1);
        assert!(active_outcome(&snapshot, 3).is_none());
    }

    #[test]
    fn search_and_keyboard_navigation_use_visible_rows() {
        let rows = plan_rows(&snapshot(), "jj");
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
}
