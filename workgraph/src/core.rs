use std::time::{SystemTime, UNIX_EPOCH};

mod patch;

use crate::contract::{
    CompletionRequirement, Edge, EditAction, EditRequest, EditResult, EvidenceKind,
    IdempotencyReceipt, Node, Plan, PlanSnapshot, ProjectGraph, SearchRequest, SearchResult,
    SessionLink, StoredProject, Walk, WalkStep,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkGraphError {
    #[error("work graph persistence failed")]
    Persistence(#[from] PersistenceError),
    #[error("work graph data could not be encoded")]
    Encoding(#[from] serde_json::Error),
    #[error("plan was not found")]
    PlanNotFound,
    #[error("node was not found")]
    NodeNotFound,
    #[error("walk was not found")]
    WalkNotFound,
    #[error("the current session is not attached to a work graph")]
    SessionNotAttached,
    #[error("the work graph has no active node")]
    NoActiveNode,
    #[error("walk is not positioned at that node")]
    PositionConflict,
    #[error("node or walk changed since it was read")]
    VersionConflict,
    #[error("idempotency key was reused for a different request")]
    IdempotencyConflict,
    #[error("edge would create a cycle")]
    DependencyCycle,
    #[error("the next node is not connected to the current node")]
    InvalidSuccessor,
    #[error("the current node has multiple outcomes; choose the next node")]
    AmbiguousSuccessor,
    #[error("completion evidence does not meet the node requirement")]
    EvidenceMismatch,
    #[error("rewind target is not on the active walk")]
    InvalidRewind,
    #[error("invalid work graph input: {0}")]
    InvalidInput(&'static str),
    #[error("system clock is unavailable")]
    Clock,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PersistenceError {
    message: String,
}

impl PersistenceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionMode {
    Read,
    Write,
}

pub trait Persistence {
    type Transaction<'a>: WorkGraphTransaction
    where
        Self: 'a;

    fn begin(&mut self, mode: TransactionMode) -> Result<Self::Transaction<'_>, PersistenceError>;
}

pub trait WorkGraphTransaction {
    fn idempotency_receipt(
        &self,
        key: &str,
    ) -> Result<Option<IdempotencyReceipt>, PersistenceError>;
    fn record_idempotency(
        &mut self,
        key: &str,
        fingerprint: &str,
        result: &EditResult,
        created_at: i64,
    ) -> Result<(), PersistenceError>;
    fn project(&self, project: &str) -> Result<Option<StoredProject>, PersistenceError>;
    fn save_project(
        &mut self,
        project: &str,
        value: &StoredProject,
        updated_at: i64,
    ) -> Result<(), PersistenceError>;
    fn commit(self) -> Result<(), PersistenceError>;
}

pub struct WorkGraph<P> {
    persistence: P,
}

impl<P: Persistence> WorkGraph<P> {
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    pub fn search(&mut self, request: &SearchRequest) -> Result<SearchResult, WorkGraphError> {
        let transaction = self.persistence.begin(TransactionMode::Read)?;
        let project = match request {
            SearchRequest::Project { project }
            | SearchRequest::Plan { project, .. }
            | SearchRequest::Node { project, .. }
            | SearchRequest::Session { project, .. } => project,
        };
        let stored = transaction.project(project)?.unwrap_or_else(StoredProject::new);
        match request {
            SearchRequest::Project { .. } => Ok(SearchResult::Project(stored.graph)),
            SearchRequest::Plan { plan, walk, .. } => Ok(SearchResult::Plan(snapshot(
                &stored.graph,
                *plan,
                *walk,
            )?)),
            SearchRequest::Node { plan, number, .. } => stored
                .graph
                .nodes
                .iter()
                .find(|node| node.plan_number == *plan && node.number == *number)
                .cloned()
                .map(SearchResult::Node)
                .ok_or(WorkGraphError::NodeNotFound),
            SearchRequest::Session { session_id, .. } => Ok(SearchResult::Session(
                stored
                    .graph
                    .sessions
                    .iter()
                    .find(|link| link.session_id == *session_id)
                    .cloned(),
            )),
        }
    }

    pub fn edit(&mut self, request: &EditRequest) -> Result<EditResult, WorkGraphError> {
        validate_request(request)?;
        let fingerprint = serde_json::to_string(request)?;
        let mut transaction = self.persistence.begin(TransactionMode::Write)?;
        if let Some(receipt) = transaction.idempotency_receipt(&request.idempotency_key)? {
            if receipt.fingerprint != fingerprint {
                return Err(WorkGraphError::IdempotencyConflict);
            }
            return Ok(receipt.result);
        }
        let now = now_ms()?;
        let mut stored = transaction
            .project(&request.project)?
            .unwrap_or_else(StoredProject::new);
        let result = apply_edit(&mut stored, request, now)?;
        transaction.save_project(&request.project, &stored, now)?;
        transaction.record_idempotency(&request.idempotency_key, &fingerprint, &result, now)?;
        transaction.commit()?;
        Ok(result)
    }
}

fn apply_edit(
    stored: &mut StoredProject,
    request: &EditRequest,
    now: i64,
) -> Result<EditResult, WorkGraphError> {
    match &request.action {
        EditAction::CreatePlan {
            title,
            root_title,
            files,
            completion,
        } => {
            let plan_number = take(&mut stored.next_plan_number);
            let node_number = take(&mut stored.next_node_number);
            let walk_number = take(&mut stored.next_walk_number);
            let plan = Plan {
                project: request.project.clone(),
                number: plan_number,
                title: title.trim().to_owned(),
                root_node: node_number,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            let root = Node {
                plan_number,
                number: node_number,
                title: root_title.trim().to_owned(),
                acceptance: String::new(),
                files: normalized_files(files),
                completion: *completion,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            let walk = Walk {
                plan_number,
                number: walk_number,
                current_node: Some(node_number),
                head_step: None,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            stored.graph.plans.push(plan.clone());
            stored.graph.nodes.push(root);
            stored.graph.walks.push(walk);
            Ok(EditResult::Plan(snapshot(
                &stored.graph,
                plan_number,
                Some(walk_number),
            )?))
        }
        EditAction::AddNode {
            plan,
            title,
            files,
            completion,
            after,
        } => {
            require_plan(&stored.graph, *plan)?;
            if let Some(after) = after {
                require_node(&stored.graph, *plan, *after)?;
            }
            let number = take(&mut stored.next_node_number);
            let node = Node {
                plan_number: *plan,
                number,
                title: title.trim().to_owned(),
                acceptance: String::new(),
                files: normalized_files(files),
                completion: *completion,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            stored.graph.nodes.push(node.clone());
            if let Some(from) = after {
                stored.graph.edges.push(Edge {
                    plan_number: *plan,
                    from: *from,
                    to: number,
                });
            }
            bump_plan(&mut stored.graph, *plan, now)?;
            Ok(EditResult::Node(node))
        }
        EditAction::SetNode {
            plan,
            number,
            title,
            files,
            completion,
            expected_version,
        } => {
            let node = stored
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.plan_number == *plan && node.number == *number)
                .ok_or(WorkGraphError::NodeNotFound)?;
            if expected_version.is_some_and(|version| version != node.version) {
                return Err(WorkGraphError::VersionConflict);
            }
            if let Some(title) = title {
                node.title = title.trim().to_owned();
            }
            if let Some(files) = files {
                node.files = normalized_files(files);
            }
            if let Some(completion) = completion {
                node.completion = *completion;
            }
            node.version = node.version.saturating_add(1);
            node.updated_at = now;
            let node = node.clone();
            bump_plan(&mut stored.graph, *plan, now)?;
            Ok(EditResult::Node(node))
        }
        EditAction::Patch { .. } => Ok(EditResult::Plan(patch::apply(stored, request, now)?)),
        EditAction::AddEdge { plan, from, to } => {
            require_node(&stored.graph, *plan, *from)?;
            require_node(&stored.graph, *plan, *to)?;
            if from == to || reaches(&stored.graph, *plan, *to, *from) {
                return Err(WorkGraphError::DependencyCycle);
            }
            let edge = Edge {
                plan_number: *plan,
                from: *from,
                to: *to,
            };
            if !stored.graph.edges.contains(&edge) {
                stored.graph.edges.push(edge);
                bump_plan(&mut stored.graph, *plan, now)?;
            }
            Ok(EditResult::Edge(edge))
        }
        EditAction::RemoveEdge { plan, from, to } => {
            let edge = Edge {
                plan_number: *plan,
                from: *from,
                to: *to,
            };
            let previous = stored.graph.edges.len();
            stored.graph.edges.retain(|candidate| *candidate != edge);
            if stored.graph.edges.len() == previous {
                return Err(WorkGraphError::InvalidSuccessor);
            }
            bump_plan(&mut stored.graph, *plan, now)?;
            Ok(EditResult::RemovedEdge(edge))
        }
        EditAction::CreateWalk { plan } => {
            let root = require_plan(&stored.graph, *plan)?.root_node;
            let number = take(&mut stored.next_walk_number);
            let walk = Walk {
                plan_number: *plan,
                number,
                current_node: Some(root),
                head_step: None,
                version: 1,
                created_at: now,
                updated_at: now,
            };
            stored.graph.walks.push(walk.clone());
            Ok(EditResult::Walk(walk))
        }
        EditAction::Advance {
            walk,
            number,
            next,
            outcome,
            expected_version,
        } => {
            let walk_index = stored
                .graph
                .walks
                .iter()
                .position(|candidate| candidate.number == *walk)
                .ok_or(WorkGraphError::WalkNotFound)?;
            Ok(EditResult::Step(advance_walk(
                stored,
                walk_index,
                *number,
                *next,
                outcome,
                *expected_version,
                now,
            )?))
        }
        EditAction::Rewind {
            walk,
            number,
            expected_version,
        } => {
            let walk_index = stored
                .graph
                .walks
                .iter()
                .position(|candidate| candidate.number == *walk)
                .ok_or(WorkGraphError::WalkNotFound)?;
            let current = &stored.graph.walks[walk_index];
            if expected_version.is_some_and(|version| version != current.version) {
                return Err(WorkGraphError::VersionConflict);
            }
            if current.current_node == Some(*number) {
                return Ok(EditResult::Walk(current.clone()));
            }
            let step = active_steps(&stored.graph, current)
                .into_iter()
                .find(|step| step.node_number == *number)
                .ok_or(WorkGraphError::InvalidRewind)?;
            let parent_step = step.parent_step;
            let current = &mut stored.graph.walks[walk_index];
            current.current_node = Some(*number);
            current.head_step = parent_step;
            current.version = current.version.saturating_add(1);
            current.updated_at = now;
            Ok(EditResult::Walk(current.clone()))
        }
        EditAction::Complete {
            session_id,
            next,
            outcome,
        } => {
            let link = stored
                .graph
                .sessions
                .iter()
                .find(|link| link.session_id == *session_id)
                .cloned()
                .ok_or(WorkGraphError::SessionNotAttached)?;
            let walk_index = stored
                .graph
                .walks
                .iter()
                .position(|walk| walk.number == link.walk_number)
                .ok_or(WorkGraphError::WalkNotFound)?;
            let number = stored.graph.walks[walk_index]
                .current_node
                .ok_or(WorkGraphError::NoActiveNode)?;
            let mut outcome = outcome.clone();
            outcome.evidence.kind =
                match require_node(&stored.graph, link.plan_number, number)?.completion {
                    CompletionRequirement::File => EvidenceKind::File,
                    CompletionRequirement::Observation
                    | CompletionRequirement::RevisionOrObservation => EvidenceKind::Observation,
                };
            advance_walk(stored, walk_index, number, *next, &outcome, None, now)?;
            Ok(EditResult::Plan(snapshot(
                &stored.graph,
                link.plan_number,
                Some(link.walk_number),
            )?))
        }
        EditAction::LinkSession {
            walk,
            session_id,
            session_path,
        } => Ok(EditResult::Session(attach_session(
            &mut stored.graph,
            *walk,
            session_id,
            session_path,
            now,
        )?)),
        EditAction::UnlinkSession { session_id } => {
            let index = stored
                .graph
                .sessions
                .iter()
                .position(|candidate| candidate.session_id == *session_id)
                .ok_or(WorkGraphError::WalkNotFound)?;
            Ok(EditResult::UnlinkedSession(
                stored.graph.sessions.remove(index),
            ))
        }
    }
}

pub(super) fn attach_session(
    graph: &mut ProjectGraph,
    walk_number: u64,
    session_id: &str,
    session_path: &str,
    now: i64,
) -> Result<SessionLink, WorkGraphError> {
    let plan_number = graph
        .walks
        .iter()
        .find(|walk| walk.number == walk_number)
        .map(|walk| walk.plan_number)
        .ok_or(WorkGraphError::WalkNotFound)?;
    let link = SessionLink {
        session_id: session_id.to_owned(),
        session_path: session_path.to_owned(),
        plan_number,
        walk_number,
        linked_at: now,
    };
    graph.sessions.retain(|link| link.session_id != session_id);
    graph.sessions.push(link.clone());
    Ok(link)
}

fn advance_walk(
    stored: &mut StoredProject,
    walk_index: usize,
    number: u64,
    next: Option<u64>,
    outcome: &crate::contract::Outcome,
    expected_version: Option<u64>,
    now: i64,
) -> Result<WalkStep, WorkGraphError> {
    let walk = &stored.graph.walks[walk_index];
    let plan_number = walk.plan_number;
    let walk_number = walk.number;
    let parent_step = walk.head_step;
    if walk.current_node != Some(number) {
        return Err(WorkGraphError::PositionConflict);
    }
    if expected_version.is_some_and(|version| version != walk.version) {
        return Err(WorkGraphError::VersionConflict);
    }
    let node = require_node(&stored.graph, plan_number, number)?;
    if !node.completion.accepts(outcome.evidence.kind) {
        return Err(WorkGraphError::EvidenceMismatch);
    }
    let successors = stored
        .graph
        .edges
        .iter()
        .filter(|edge| edge.plan_number == plan_number && edge.from == number)
        .map(|edge| edge.to)
        .collect::<Vec<_>>();
    let next = choose_successor(&successors, next)?;
    let step = WalkStep {
        id: take(&mut stored.next_step_id),
        walk_number,
        node_number: number,
        parent_step,
        outcome: outcome.clone(),
        completed_at: now,
    };
    stored.graph.steps.push(step.clone());
    let walk = &mut stored.graph.walks[walk_index];
    walk.current_node = next;
    walk.head_step = Some(step.id);
    walk.version = walk.version.saturating_add(1);
    walk.updated_at = now;
    Ok(step)
}

fn snapshot(
    graph: &ProjectGraph,
    plan_number: u64,
    walk_number: Option<u64>,
) -> Result<PlanSnapshot, WorkGraphError> {
    let plan = require_plan(graph, plan_number)?.clone();
    let walk = walk_number
        .map(|number| {
            graph
                .walks
                .iter()
                .find(|walk| walk.plan_number == plan_number && walk.number == number)
                .cloned()
                .ok_or(WorkGraphError::WalkNotFound)
        })
        .transpose()?
        .or_else(|| {
            graph
                .walks
                .iter()
                .filter(|walk| walk.plan_number == plan_number)
                .max_by_key(|walk| walk.number)
                .cloned()
        });
    let walk_number = walk.as_ref().map(|walk| walk.number);
    Ok(PlanSnapshot {
        plan,
        nodes: graph
            .nodes
            .iter()
            .filter(|node| node.plan_number == plan_number)
            .cloned()
            .collect(),
        edges: graph
            .edges
            .iter()
            .filter(|edge| edge.plan_number == plan_number)
            .copied()
            .collect(),
        steps: graph
            .steps
            .iter()
            .filter(|step| Some(step.walk_number) == walk_number)
            .cloned()
            .collect(),
        sessions: graph
            .sessions
            .iter()
            .filter(|link| Some(link.walk_number) == walk_number)
            .cloned()
            .collect(),
        walk,
    })
}

fn require_plan(graph: &ProjectGraph, number: u64) -> Result<&Plan, WorkGraphError> {
    graph
        .plans
        .iter()
        .find(|plan| plan.number == number)
        .ok_or(WorkGraphError::PlanNotFound)
}

fn require_node(
    graph: &ProjectGraph,
    plan: u64,
    number: u64,
) -> Result<&Node, WorkGraphError> {
    graph
        .nodes
        .iter()
        .find(|node| node.plan_number == plan && node.number == number)
        .ok_or(WorkGraphError::NodeNotFound)
}

fn bump_plan(graph: &mut ProjectGraph, number: u64, now: i64) -> Result<(), WorkGraphError> {
    let plan = graph
        .plans
        .iter_mut()
        .find(|plan| plan.number == number)
        .ok_or(WorkGraphError::PlanNotFound)?;
    plan.version = plan.version.saturating_add(1);
    plan.updated_at = now;
    Ok(())
}

fn choose_successor(successors: &[u64], requested: Option<u64>) -> Result<Option<u64>, WorkGraphError> {
    match (successors, requested) {
        ([], None) => Ok(None),
        ([], Some(_)) => Err(WorkGraphError::InvalidSuccessor),
        ([only], None) => Ok(Some(*only)),
        (_, None) => Err(WorkGraphError::AmbiguousSuccessor),
        (_, Some(next)) if successors.contains(&next) => Ok(Some(next)),
        _ => Err(WorkGraphError::InvalidSuccessor),
    }
}

fn reaches(graph: &ProjectGraph, plan: u64, from: u64, target: u64) -> bool {
    let mut pending = vec![from];
    let mut seen = std::collections::HashSet::new();
    while let Some(number) = pending.pop() {
        if !seen.insert(number) {
            continue;
        }
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.plan_number == plan && edge.from == number)
        {
            if edge.to == target {
                return true;
            }
            pending.push(edge.to);
        }
    }
    false
}

fn active_steps<'a>(graph: &'a ProjectGraph, walk: &Walk) -> Vec<&'a WalkStep> {
    let mut result = Vec::new();
    let mut current = walk.head_step;
    while let Some(id) = current {
        let Some(step) = graph
            .steps
            .iter()
            .find(|step| step.walk_number == walk.number && step.id == id)
        else {
            break;
        };
        result.push(step);
        current = step.parent_step;
    }
    result
}

fn normalized_files(files: &[String]) -> Vec<String> {
    let mut result = files
        .iter()
        .map(|file| file.trim())
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

fn validate_request(request: &EditRequest) -> Result<(), WorkGraphError> {
    if request.project.trim().is_empty()
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 256
    {
        return Err(WorkGraphError::InvalidInput(
            "project and idempotency key are required",
        ));
    }
    let valid_title = |title: &str| !title.trim().is_empty() && title.len() <= 512;
    let valid_files = |files: &[String]| {
        files.len() <= 64
            && files
                .iter()
                .all(|file| !file.trim().is_empty() && file.len() <= 4096)
    };
    match &request.action {
        EditAction::CreatePlan {
            title,
            root_title,
            files,
            ..
        } if !valid_title(title) || !valid_title(root_title) || !valid_files(files) => {
            Err(WorkGraphError::InvalidInput("plan fields are invalid"))
        }
        EditAction::AddNode { title, files, .. }
            if !valid_title(title) || !valid_files(files) =>
        {
            Err(WorkGraphError::InvalidInput("node fields are invalid"))
        }
        EditAction::SetNode {
            title,
            files,
            completion,
            ..
        } if title
            .as_ref()
            .is_some_and(|title| !valid_title(title))
            || files.as_ref().is_some_and(|files| !valid_files(files))
            || (title.is_none() && files.is_none() && completion.is_none()) =>
        {
            Err(WorkGraphError::InvalidInput("node fields are invalid"))
        }
        EditAction::Patch {
            nodes,
            session_id,
            session_path,
            ..
        } if nodes.is_empty()
            || nodes.len() > 64
            || nodes.iter().any(|node| {
                !valid_title(&node.title)
                    || node.acceptance.trim().is_empty()
                    || node.acceptance.len() > 4096
            })
            || session_id.trim().is_empty()
            || session_id.len() > 256
            || session_path.trim().is_empty()
            || session_path.len() > 4096 =>
        {
            Err(WorkGraphError::InvalidInput("node patch is invalid"))
        }
        EditAction::Advance { outcome, .. } | EditAction::Complete { outcome, .. }
            if outcome.note.trim().is_empty()
                || outcome.note.len() > 4096
                || outcome.evidence.reference.trim().is_empty()
                || outcome.evidence.reference.len() > 4096 =>
        {
            Err(WorkGraphError::InvalidInput("completion outcome is invalid"))
        }
        EditAction::Complete { session_id, .. }
            if session_id.trim().is_empty() || session_id.len() > 256 =>
        {
            Err(WorkGraphError::InvalidInput("session link is invalid"))
        }
        EditAction::LinkSession {
            session_id,
            session_path,
            ..
        } if session_id.trim().is_empty()
            || session_id.len() > 256
            || session_path.trim().is_empty()
            || session_path.len() > 4096 =>
        {
            Err(WorkGraphError::InvalidInput("session link is invalid"))
        }
        EditAction::UnlinkSession { session_id }
            if session_id.trim().is_empty() || session_id.len() > 256 =>
        {
            Err(WorkGraphError::InvalidInput("session link is invalid"))
        }
        _ => Ok(()),
    }
}

fn take(next: &mut u64) -> u64 {
    let value = (*next).max(1);
    *next = value.saturating_add(1);
    value
}

fn now_ms() -> Result<i64, WorkGraphError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WorkGraphError::Clock)?
        .as_millis();
    i64::try_from(millis).map_err(|_| WorkGraphError::Clock)
}
