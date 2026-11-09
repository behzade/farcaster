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

#[gpui::test]
fn plan_titles_keep_a_shared_column_for_numbers_and_completed_markers(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{point, px, size};
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let rows = [7, 725, 1024].map(|number| PlanRow {
        node: workgraph::Node {
            number,
            plan_number: 1,
            title: "A task with a long title that wraps onto the next line".into(),
            acceptance: String::new(),
            files: Vec::new(),
            completion: Default::default(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        },
        reached: number == 7,
        current: number == 725,
        detached: false,
    });
    for width in [220.0, 332.0, 450.0] {
        let marker_width = plan_marker_width(&rows);
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(width), px(200.0)),
            |_, _| {
                div().w(px(width)).flex().flex_col().children(
                    rows.iter()
                        .cloned()
                        .map(|row| sidebar_row_content(row, marker_width)),
                )
            },
        );
        let mut column = None;
        for (marker, title) in [
            ("plan-marker-7", "plan-title-7"),
            ("plan-marker-725", "plan-title-725"),
            ("plan-marker-1024", "plan-title-1024"),
        ] {
            let marker = cx.debug_bounds(marker).unwrap();
            let title = cx.debug_bounds(title).unwrap();
            assert_eq!(*column.get_or_insert(title.left()), title.left());
            assert!(title.left() - marker.right() >= theme().space.sm);
            assert!(title.right() <= px(width));
            assert!(title.size.width > px(0.0));
            assert_eq!(title.top(), marker.top());
        }
    }
}
