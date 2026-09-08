use std::collections::{HashMap, HashSet, VecDeque};

use workgraph::PlanSnapshot;

use super::contract::PlanRow;

pub(super) fn plan_rows(snapshot: &PlanSnapshot, search: &str) -> Vec<PlanRow> {
    let reached = snapshot
        .active_steps()
        .into_iter()
        .map(|step| step.node_number)
        .collect::<HashSet<_>>();
    let current = snapshot.walk.as_ref().and_then(|walk| walk.current_node);
    let reachable = reachable_nodes(snapshot);
    topological_numbers(snapshot)
        .into_iter()
        .filter_map(|number| {
            let node = snapshot.nodes.iter().find(|node| node.number == number)?;
            node_matches(node, search).then(|| PlanRow {
                node: node.clone(),
                reached: reached.contains(&number),
                current: current == Some(number),
                detached: !reachable.contains(&number),
            })
        })
        .collect()
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

fn node_matches(node: &workgraph::Node, search: &str) -> bool {
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

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
