use crate::ProjectGraph;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkStatus {
    Active,
    Blocked,
    Ready,
    Done,
}

impl WorkStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "In progress",
            Self::Blocked => "Blocked",
            Self::Ready => "Ready",
            Self::Done => "Done",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanOverview {
    pub number: u64,
    pub status: WorkStatus,
    pub total: usize,
    pub done: usize,
    pub blocked: usize,
    pub updated_at: i64,
}

impl ProjectGraph {
    pub fn work_status(&self, task: u64) -> WorkStatus {
        let state = self.task_state(task);
        if state
            .as_ref()
            .is_some_and(|state| state.completion.is_some())
        {
            return WorkStatus::Done;
        }
        if self
            .edges
            .iter()
            .filter(|edge| edge.to == task)
            .any(|edge| {
                self.task_state(edge.from)
                    .is_none_or(|state| state.completion.is_none())
            })
        {
            return WorkStatus::Blocked;
        }
        if state.as_ref().is_some_and(|state| state.owner.is_some())
            || self
                .walks
                .iter()
                .any(|walk| walk.head_step.is_some() && walk.current_node == Some(task))
        {
            return WorkStatus::Active;
        }
        WorkStatus::Ready
    }

    pub fn plan_overviews(&self) -> Vec<PlanOverview> {
        self.plans
            .iter()
            .map(|plan| {
                let mut summary = PlanOverview {
                    number: plan.number,
                    status: WorkStatus::Ready,
                    total: 0,
                    done: 0,
                    blocked: 0,
                    updated_at: plan.updated_at,
                };
                let mut active = false;
                for node in self
                    .nodes
                    .iter()
                    .filter(|node| node.plan_number == plan.number)
                {
                    summary.total += 1;
                    summary.updated_at = summary.updated_at.max(node.updated_at);
                    match self.work_status(node.number) {
                        WorkStatus::Done => summary.done += 1,
                        WorkStatus::Blocked => summary.blocked += 1,
                        WorkStatus::Active => active = true,
                        WorkStatus::Ready => {}
                    }
                    if let Some(state) = self.task_state(node.number) {
                        if let Some(owner) = state.owner {
                            summary.updated_at = summary.updated_at.max(owner.claimed_at);
                        }
                        if let Some(completion) = state.completion {
                            summary.updated_at = summary.updated_at.max(completion.completed_at);
                        }
                    }
                }
                for walk in self
                    .walks
                    .iter()
                    .filter(|walk| walk.plan_number == plan.number)
                {
                    summary.updated_at = summary.updated_at.max(walk.updated_at);
                }
                summary.status = if summary.total > 0 && summary.done == summary.total {
                    WorkStatus::Done
                } else if active
                    || summary.done > 0 && summary.done + summary.blocked < summary.total
                {
                    WorkStatus::Active
                } else if summary.blocked > 0 && summary.done + summary.blocked == summary.total {
                    WorkStatus::Blocked
                } else {
                    WorkStatus::Ready
                };
                summary
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
