use super::*;
use gpui::px;

#[gpui::test]
fn transcript_and_composer_keep_the_same_content_edges(cx: &mut gpui::TestAppContext) {
    use gpui::{InteractiveElement as _, ParentElement as _, point, size};
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for width in [320.0, 640.0, 1400.0] {
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(width), px(200.0)),
            |_, _| {
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .child(
                        transcript_column().child(
                            div().px(super::super::grid::transcript_inset()).child(
                                div()
                                    .debug_selector(|| "message-content".into())
                                    .h(px(20.0)),
                            ),
                        ),
                    )
                    .child(
                        chat_column().child(
                            div()
                                .debug_selector(|| "composer-content".into())
                                .h(px(20.0)),
                        ),
                    )
            },
        );
        let message = cx.debug_bounds("message-content").unwrap();
        let composer = cx.debug_bounds("composer-content").unwrap();
        assert_eq!(message.left(), composer.left());
        assert_eq!(message.right(), composer.right());
        assert!(message.size.width > px(0.0));
        assert_eq!(message.size.width, px((width - 80.0).min(950.0)));
        assert_eq!(
            message.left(),
            px((width - f32::from(message.size.width)) / 2.0)
        );
    }
}

#[test]
fn composer_clearance_adapts_and_stays_bounded() {
    assert_eq!(composer_bottom_clearance(px(300.0)), px(12.0));
    assert_eq!(composer_bottom_clearance(px(600.0)), px(12.0));
    assert_eq!(composer_bottom_clearance(px(800.0)), px(24.0));
    assert_eq!(composer_bottom_clearance(px(2000.0)), px(28.0));
}

#[test]
fn exact_layout_boundaries_are_stable() {
    assert_eq!(layout_mode(px(959.0)), LayoutMode::Narrow);
    assert_eq!(layout_mode(px(960.0)), LayoutMode::Compact);
    assert_eq!(layout_mode(px(1_179.0)), LayoutMode::Compact);
    assert_eq!(layout_mode(px(1_180.0)), LayoutMode::Wide);
}

#[test]
fn compact_moves_only_the_right_panel_and_narrow_moves_both() {
    assert!(shows_left_inline(LayoutMode::Wide));
    assert!(shows_right_inline(LayoutMode::Wide));
    assert!(!shows_run_sheet_button(LayoutMode::Wide));

    assert!(shows_left_inline(LayoutMode::Compact));
    assert!(!shows_right_inline(LayoutMode::Compact));
    assert!(!shows_session_sheet_button(LayoutMode::Compact));
    assert!(shows_run_sheet_button(LayoutMode::Compact));

    assert!(!shows_left_inline(LayoutMode::Narrow));
    assert!(!shows_right_inline(LayoutMode::Narrow));
    assert!(shows_session_sheet_button(LayoutMode::Narrow));
    assert!(shows_run_sheet_button(LayoutMode::Narrow));
}
