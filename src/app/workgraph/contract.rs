use workgraph::contract::{Plan, PlanSnapshot, SessionLink};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct PlanData {
    pub plans: Vec<Plan>,
    pub snapshot: Option<PlanSnapshot>,
    pub session_link: Option<SessionLink>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanLoadState {
    Loading,
    Ready(Box<PlanData>),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanRow {
    pub node: workgraph::contract::Node,
    pub reached: bool,
    pub current: bool,
    pub detached: bool,
}
