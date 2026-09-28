use rusqlite::{Connection, params};

use crate::{PersistenceError, ProjectGraph, StoredProject};

pub fn remap_session_keys(
    connection: &Connection,
    resolve: impl Fn(&str, &str, Option<&str>) -> Option<String>,
) -> Result<(), PersistenceError> {
    update_graphs(connection, |project, graph| {
        let before = graph.clone();
        for link in &mut graph.sessions {
            if let Some(key) = resolve(project, &link.session_id, Some(&link.session_path)) {
                link.session_id = key;
            }
        }
        for task in &mut graph.tasks {
            if let Some(owner) = &mut task.owner
                && let Some(key) = resolve(project, &owner.session_id, Some(&owner.session_path))
            {
                owner.session_id = key;
            }
            if let Some(completion) = &mut task.completion
                && let Some(key) = resolve(project, &completion.session_id, None)
            {
                completion.session_id = key;
            }
        }
        if *graph != before {
            coalesce_links(graph);
        }
    })
}

pub fn delete_session_keys(
    connection: &Connection,
    keys: &[String],
    now: i64,
) -> Result<(), PersistenceError> {
    if keys.is_empty() {
        return Ok(());
    }
    update_graphs(connection, |_, graph| {
        let removed_walks: Vec<_> = graph
            .sessions
            .iter()
            .filter(|link| keys.contains(&link.session_id))
            .map(|link| link.walk_number)
            .collect();
        let completed: Vec<_> = graph
            .nodes
            .iter()
            .filter_map(|node| graph.task_state(node.number))
            .filter(|task| {
                task.completion
                    .as_ref()
                    .is_some_and(|completion| keys.contains(&completion.session_id))
            })
            .collect();
        for state in completed {
            if let Some(task) = graph.tasks.iter_mut().find(|task| task.task == state.task) {
                task.completion = state.completion;
            } else {
                graph.tasks.push(state);
            }
        }
        let mut released = Vec::new();
        for task in &mut graph.tasks {
            if task
                .owner
                .as_ref()
                .is_some_and(|owner| keys.contains(&owner.session_id))
            {
                task.owner = None;
                released.push(task.task);
            }
        }
        graph
            .sessions
            .retain(|link| !keys.contains(&link.session_id));
        for walk in &mut graph.walks {
            let detached = removed_walks.contains(&walk.number)
                && !graph
                    .sessions
                    .iter()
                    .any(|link| link.walk_number == walk.number);
            if walk
                .current_node
                .is_some_and(|task| detached || released.contains(&task))
            {
                walk.current_node = None;
                walk.version = walk.version.saturating_add(1);
                walk.updated_at = now;
            }
        }
    })
}

fn update_graphs(
    connection: &Connection,
    update: impl Fn(&str, &mut ProjectGraph),
) -> Result<(), PersistenceError> {
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='wg_plan_store')",
            [],
            |row| row.get(0),
        )
        .map_err(error)?;
    if !exists {
        return Ok(());
    }
    let rows = connection
        .prepare("SELECT project, data_json FROM wg_plan_store")
        .map_err(error)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    for (project, json) in rows {
        let mut stored: StoredProject = serde_json::from_str(&json).map_err(error)?;
        let before = stored.graph.clone();
        update(&project, &mut stored.graph);
        if stored.graph != before {
            connection
                .execute(
                    "UPDATE wg_plan_store SET data_json=?2 WHERE project=?1",
                    params![project, serde_json::to_string(&stored).map_err(error)?],
                )
                .map_err(error)?;
        }
    }
    Ok(())
}

fn coalesce_links(graph: &mut ProjectGraph) {
    let mut links = std::mem::take(&mut graph.sessions);
    links.sort_by_key(|link| {
        let active = graph.walks.iter().any(|walk| {
            walk.number == link.walk_number
                && walk.current_node.is_some_and(|task| {
                    graph.tasks.iter().any(|state| {
                        state.task == task
                            && state
                                .owner
                                .as_ref()
                                .is_some_and(|owner| owner.session_id == link.session_id)
                    })
                })
        });
        std::cmp::Reverse((active, link.linked_at, link.walk_number))
    });
    let mut discarded_walks = Vec::new();
    for link in links {
        if !graph
            .sessions
            .iter()
            .any(|other| other.session_id == link.session_id)
        {
            graph.sessions.push(link);
        } else {
            discarded_walks.push(link.walk_number);
        }
    }
    for number in discarded_walks {
        if !graph.sessions.iter().any(|kept| kept.walk_number == number)
            && let Some(walk) = graph.walks.iter_mut().find(|walk| walk.number == number)
        {
            if walk.current_node.take().is_some() {
                walk.version = walk.version.saturating_add(1);
            }
        }
    }
}

fn error(value: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::new(value.to_string())
}

#[cfg(test)]
#[path = "session_identity_tests.rs"]
mod tests;
