use super::*;
use gpui::{point, px, size};

#[gpui::test]
fn session_project_and_actions_stay_above_title(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for width in [220.0, 280.0, 400.0] {
        for compact in [false, true, false] {
            cx.draw(
                point(px(0.0), px(0.0)),
                size(px(width), px(100.0)),
                |_, _| {
                    div()
                        .w(px(width))
                        .h(session_row_height(compact))
                        .flex()
                        .child(session_row_content(
                            compact,
                            session_title(
                                "A long session title that needs the available width ".repeat(3),
                                false,
                                false,
                            )
                            .debug_selector(|| "title".into())
                            .into_any_element(),
                            div()
                                .debug_selector(|| "project".into())
                                .min_w_0()
                                .flex_1()
                                .h(px(20.0))
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child("Project")
                                .into_any_element(),
                            div()
                                .debug_selector(|| "actions".into())
                                .flex_none()
                                .w(px(100.0))
                                .h(theme().controls.icon_button)
                                .into_any_element(),
                        ))
                },
            );
            let title = cx.debug_bounds("title").expect("title");
            let actions = cx.debug_bounds("actions").expect("actions");
            assert!(title.size.height <= theme().type_scale.line_body);
            assert!(actions.right() <= px(width));
            assert!(actions.bottom() <= session_row_height(compact));
            let project = cx.debug_bounds("project").expect("project");
            assert_eq!(title.size.width, px(width));
            assert!(actions.bottom() <= title.top());
            assert!(project.bottom() <= title.top());
            assert!(project.right() <= actions.left());
        }
    }
}
