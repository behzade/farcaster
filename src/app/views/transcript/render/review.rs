use crate::app::{
    FarcasterApp, reviews::Review, ui::theme::THEME, views::transcript::tool_changes,
};
use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

use super::review_artifact::Artifact;

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    font_scale: f32,
    key: usize,
    artifact: Artifact,
    expanded: bool,
    working: bool,
    continued: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    use crate::app::ui::{
        file_icons::file_icon,
        theme::{MONO_FONT_FAMILY, UI_FONT_FAMILY},
    };
    let review = artifact.review;
    let project = artifact.project;
    let open_review = review.clone();
    let open_project = project.clone();
    let open_entity = entity.clone();
    div()
        .w_full()
        .px(super::TRANSCRIPT_HORIZONTAL_PADDING)
        .py(THEME.space.xs)
        .font_family(UI_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small * font_scale)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(review_header(
            key,
            review.title.clone(),
            review.items.len(),
            expanded,
            super::toggle_transcript_item(entity.clone(), key, expanded),
            move |window, cx| {
                let _ = open_entity.update(cx, |this, cx| {
                    this.open_review_editor(open_project.clone(), open_review.clone(), window, cx);
                });
            },
        ))
        .when(working || continued, |row| {
            row.child(div().text_color(THEME.colors.muted).child(if working {
                "Agent still working"
            } else {
                "Submitted before the agent finished"
            }))
        })
        .when(expanded, |row| {
            row.child(
                div()
                    .pl(THEME.space.sm)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .children(review.items.iter().enumerate().map(|(index, location)| {
                        let line = location
                            .start_line
                            .map_or(String::new(), |line| match location.end_line {
                                Some(end) => format!(":{line}–{end}"),
                                None => format!(":{line}"),
                            });
                        let path = format!("{}{line}", location.path);
                        let single = Review {
                            title: review.title.clone(),
                            items: vec![location.clone()],
                        };
                        let project = project.clone();
                        let entity = entity.clone();
                        tool_changes::title_row(
                            format!("review-location-{key}-{index}"),
                            format!("{path} — {}", location.note),
                            move |window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.open_review_editor(
                                        project.clone(),
                                        single.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            },
                        )
                        .flex_col()
                        .items_start()
                        .py(THEME.space.xs)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .child(file_icon(std::path::Path::new(&location.path)))
                                .child(
                                    div()
                                        .font_family(MONO_FONT_FAMILY)
                                        .text_color(THEME.colors.text)
                                        .child(path),
                                ),
                        )
                        .child(
                            div()
                                .pl(THEME.icons.inline + THEME.space.xs)
                                .text_color(THEME.colors.muted)
                                .child(location.note.clone()),
                        )
                    }))
                    .child(div().text_color(THEME.colors.muted).child(
                        "Suggested locations, not a verified changeset. Line ranges may be stale.",
                    )),
            )
        })
        .into_any_element()
}

fn review_header(
    key: usize,
    title: String,
    count: usize,
    expanded: bool,
    toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
    open: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    use crate::app::ui::{
        assets::AppIcon,
        primitives::{AppIconSize, activates_button, app_icon, icon_control},
    };
    let open = std::rc::Rc::new(open);
    let click = open.clone();
    tool_changes::title_row(
        format!("review-header-{key}"),
        format!(
            "{} {title}, {count} locations",
            if expanded { "Hide" } else { "Show" }
        ),
        toggle,
    )
    .aria_expanded(expanded)
    .debug_selector(move || format!("review-header-{key}"))
    .gap(THEME.space.sm)
    .child(
        div()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .text_color(THEME.colors.code)
            .child(title),
    )
    .child(
        div()
            .flex_none()
            .text_color(THEME.colors.muted)
            .child(format!(
                "{count} {}",
                if count == 1 { "location" } else { "locations" }
            )),
    )
    .child(
        icon_control(format!("review-open-{key}"), "Open review in Neovim")
            .debug_selector(move || format!("review-open-{key}"))
            .text_color(THEME.colors.text)
            .hover(|control| control.bg(THEME.colors.hover))
            .child(app_icon(AppIcon::Neovim, AppIconSize::Control))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                click(window, cx);
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    open(window, cx);
                }
            }),
    )
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
