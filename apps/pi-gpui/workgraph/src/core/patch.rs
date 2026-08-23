use crate::contract::{
    CompletionRequirement, Edge, EditAction, EditRequest, Node, NodeDraft, Plan, PlanSnapshot,
    StoredProject, Walk,
};

use super::{
    WorkGraphError, attach_session, bump_plan, require_node, require_plan, snapshot, take,
};

pub(super) fn apply(
    stored: &mut StoredProject,
    request: &EditRequest,
    now: i64,
) -> Result<PlanSnapshot, WorkGraphError> {
    let EditAction::Patch {
        nodes,
        after,
        before,
        session_id,
        session_path,
    } = &request.action
    else {
        return Err(WorkGraphError::InvalidInput("node patch is invalid"));
    };
    let first_draft = nodes
        .first()
        .ok_or(WorkGraphError::InvalidInput("node patch is empty"))?;
    let (after, before) = (*after, *before);

    if after.is_none() && before.is_none() {
        let plan_number = take(&mut stored.next_plan_number);
        let (root, _) = append_node_chain(stored, plan_number, nodes, now)?;
        let walk_number = take(&mut stored.next_walk_number);
        stored.graph.plans.push(Plan {
            project: request.project.clone(),
            number: plan_number,
            title: first_draft.title.trim().to_owned(),
            root_node: root,
            version: 1,
            created_at: now,
            updated_at: now,
        });
        stored.graph.walks.push(Walk {
            plan_number,
            number: walk_number,
            current_node: Some(root),
            head_step: None,
            version: 1,
            created_at: now,
            updated_at: now,
        });
        attach_session(
            &mut stored.graph,
            walk_number,
            session_id,
            session_path,
            now,
        )?;
        return snapshot(&stored.graph, plan_number, Some(walk_number));
    }

    let plan_number = after
        .or(before)
        .and_then(|number| {
            stored
                .graph
                .nodes
                .iter()
                .find(|node| node.number == number)
                .map(|node| node.plan_number)
        })
        .ok_or(WorkGraphError::NodeNotFound)?;
    let link = stored
        .graph
        .sessions
        .iter()
        .find(|link| link.session_id == *session_id)
        .cloned()
        .ok_or(WorkGraphError::SessionNotAttached)?;
    if link.plan_number != plan_number {
        return Err(WorkGraphError::InvalidInput(
            "patch points are outside the attached graph",
        ));
    }
    if let Some(number) = after {
        require_node(&stored.graph, plan_number, number)?;
    }
    if let Some(number) = before {
        require_node(&stored.graph, plan_number, number)?;
    }
    if after == before {
        return Err(WorkGraphError::InvalidInput("patch points must differ"));
    }
    if let (Some(from), Some(to)) = (after, before) {
        let edge = Edge {
            plan_number,
            from,
            to,
        };
        let index = stored
            .graph
            .edges
            .iter()
            .position(|candidate| *candidate == edge)
            .ok_or(WorkGraphError::InvalidSuccessor)?;
        stored.graph.edges.remove(index);
    } else if let Some(before) = before
        && require_plan(&stored.graph, plan_number)?.root_node != before
    {
        return Err(WorkGraphError::InvalidInput(
            "a patch without after must attach before the graph root",
        ));
    }

    let (first, last) = append_node_chain(stored, plan_number, nodes, now)?;
    if let Some(from) = after {
        stored.graph.edges.push(Edge {
            plan_number,
            from,
            to: first,
        });
    }
    if let Some(to) = before {
        stored.graph.edges.push(Edge {
            plan_number,
            from: last,
            to,
        });
    }

    if after.is_none() {
        let old_root = before.ok_or(WorkGraphError::InvalidInput("patch point is missing"))?;
        let plan = stored
            .graph
            .plans
            .iter_mut()
            .find(|plan| plan.number == plan_number)
            .ok_or(WorkGraphError::PlanNotFound)?;
        plan.root_node = first;
        for walk in stored.graph.walks.iter_mut().filter(|walk| {
            walk.plan_number == plan_number
                && walk.head_step.is_none()
                && walk.current_node == Some(old_root)
        }) {
            walk.current_node = Some(first);
            walk.version = walk.version.saturating_add(1);
            walk.updated_at = now;
        }
    } else if let Some(after) = after {
        let move_walks = stored
            .graph
            .walks
            .iter()
            .filter(|walk| walk.plan_number == plan_number && walk.current_node == before)
            .filter(|walk| {
                walk.head_step.is_some_and(|id| {
                    stored.graph.steps.iter().any(|step| {
                        step.id == id
                            && step.walk_number == walk.number
                            && step.node_number == after
                    })
                })
            })
            .map(|walk| walk.number)
            .collect::<Vec<_>>();
        for walk in stored
            .graph
            .walks
            .iter_mut()
            .filter(|walk| move_walks.contains(&walk.number))
        {
            walk.current_node = Some(first);
            walk.version = walk.version.saturating_add(1);
            walk.updated_at = now;
        }
    }

    bump_plan(&mut stored.graph, plan_number, now)?;
    attach_session(
        &mut stored.graph,
        link.walk_number,
        session_id,
        session_path,
        now,
    )?;
    snapshot(&stored.graph, plan_number, Some(link.walk_number))
}

fn append_node_chain(
    stored: &mut StoredProject,
    plan_number: u64,
    drafts: &[NodeDraft],
    now: i64,
) -> Result<(u64, u64), WorkGraphError> {
    let mut first = None;
    let mut previous = None;
    for draft in drafts {
        let number = take(&mut stored.next_node_number);
        first.get_or_insert(number);
        if let Some(from) = previous {
            stored.graph.edges.push(Edge {
                plan_number,
                from,
                to: number,
            });
        }
        stored.graph.nodes.push(Node {
            plan_number,
            number,
            title: draft.title.trim().to_owned(),
            acceptance: draft.acceptance.trim().to_owned(),
            files: Vec::new(),
            completion: CompletionRequirement::RevisionOrObservation,
            version: 1,
            created_at: now,
            updated_at: now,
        });
        previous = Some(number);
    }
    first
        .zip(previous)
        .ok_or(WorkGraphError::InvalidInput("node patch is empty"))
}
