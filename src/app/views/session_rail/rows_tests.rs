use super::*;
use gpui::{point, px, size};

#[gpui::test]
fn flat_rows_give_titles_the_full_width_and_grouped_rows_share_one_line(
    cx: &mut gpui::TestAppContext,
) {
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
                            div()
                                .debug_selector(|| "title".into())
                                .w_full()
                                .h(px(20.0))
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child("A long session title that needs the available width")
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
            assert!(actions.right() <= px(width));
            assert!(actions.bottom() <= session_row_height(compact));
            if compact {
                assert!(cx.debug_bounds("project").is_none());
                assert!(title.right() <= actions.left());
                assert!(title.top() < actions.bottom() && title.bottom() > actions.top());
            } else {
                let project = cx.debug_bounds("project").expect("project");
                assert_eq!(title.size.width, px(width));
                assert!(title.bottom() <= actions.top());
                assert!(title.bottom() <= project.top());
                assert!(project.right() <= actions.left());
            }
        }
    }
}
