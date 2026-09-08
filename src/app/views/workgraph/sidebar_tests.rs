use workgraph::SessionLink;

use super::*;
use crate::app::views::workgraph::contract::PlanData;

fn state(attached: bool) -> PlanLoadState {
    PlanLoadState::Ready(Box::new(PlanData {
        session_link: attached.then_some(SessionLink {
            session_id: "session-1".into(),
            session_path: "/sessions/1.jsonl".into(),
            plan_number: 1,
            walk_number: 1,
            linked_at: 0,
        }),
        ..PlanData::default()
    }))
}

#[test]
fn sidebar_requires_a_real_session_link() {
    assert!(sidebar_visible(&state(true), true));
    assert!(!sidebar_visible(&state(false), true));
    assert!(!sidebar_visible(&state(true), false));
}
