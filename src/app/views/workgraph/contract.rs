pub(super) type PlanData = workgraph::ProjectSelection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanLoadState {
    Loading,
    Ready(Box<PlanData>),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanRow {
    pub node: workgraph::Node,
    pub reached: bool,
    pub current: bool,
    pub detached: bool,
}
