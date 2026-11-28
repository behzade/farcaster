use super::*;
use gpui::{point, px, size};

#[gpui::test]
fn grouped_rows_use_one_line_and_flat_rows_put_the_title_below_metadata(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    for width in [220.0, 280.0, 400.0] {
        for grouped in [false, true] {
            cx.draw(
                point(px(0.0), px(0.0)),
                size(px(width), px(100.0)),
                |_, _| {
                    div()
                        .w(px(width))
                        .h(session_row_height(grouped))
                        .flex()
                        .child(session_row_content(
                            grouped,
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
                                .h(px(20.0))
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child("A project name too long for the row")
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
            assert!(title.size.width > px(0.0));
            assert!(actions.right() <= px(width));
            assert!(actions.bottom() <= session_row_height(grouped));
            if grouped {
                assert!(cx.debug_bounds("project").is_none());
                assert_eq!(title.left(), px(0.0));
                assert!(title.right() <= actions.left());
                assert!(title.top() < actions.bottom() && title.bottom() > actions.top());
            } else {
                let project = cx.debug_bounds("project").expect("project");
                assert!(project.right() <= actions.left());
                assert!(title.top() >= actions.bottom());
                assert_eq!(title.left(), project.left());
                assert!(
                    title.size.width > actions.left(),
                    "flat titles use the full row width"
                );
            }
        }
    }
}
