use std::{collections::BTreeSet, path::Path};

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use crate::app::{
    FarcasterApp,
    ui::{
        assets::AppIcon,
        change_tree::{self, ChangeTreeState, TreeRow},
        file_icons::file_icon,
        primitives::{ButtonTone, activates_button, icon_button, preserve_pointer_focus},
        theme::THEME,
    },
    views::regions::RunPanelView,
    workspace::review::ActiveReview,
};

pub(in crate::app::views) fn render(
    active: &ActiveReview,
    scroll: &ScrollHandle,
    tree: &ChangeTreeState,
    entity: WeakEntity<FarcasterApp>,
    panel: WeakEntity<RunPanelView>,
) -> AnyElement {
    let close = entity.clone();
    // One leaf per file; all ranges remain available in selection details.
    let mut seen = BTreeSet::new();
    let rows = change_tree::rows(
        active
            .review
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| seen.insert(&item.path))
            .map(|(index, item)| (index, Path::new(&item.path), None, None)),
        "",
        &active.project,
        tree,
    );
    let selected = active.navigation.as_ref().and_then(|nav| nav.selected);
    let inspecting = active.inspecting;
    let selected_path = inspecting.map(|index| &active.review.items[index].path);
    div()
        .size_full()
        .flex()
        .flex_col()
        .p(THEME.space.md)
        .gap(THEME.space.sm)
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(THEME.colors.code)
                        .child(active.review.title.clone()),
                )
                .child(icon_button(
                    "close-review",
                    AppIcon::X,
                    "Close review",
                    ButtonTone::Quiet,
                    move |window, cx| {
                        let _ = close.update(cx, |this, cx| this.close_review(window, cx));
                    },
                )),
        )
        .when_some(active.error.as_ref(), |panel, error| {
            panel.child(div().text_color(THEME.colors.error).child(error.clone()))
        })
        .child(
            div()
                .id("review-files")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .children(rows.into_iter().map(|row| match row {
                    TreeRow::Folder {
                        path,
                        label,
                        count,
                        counts,
                        depth,
                        open,
                    } => {
                        let project = active.project.clone();
                        let accessible = format!(
                            "{} {}",
                            if open { "Collapse" } else { "Expand" },
                            path.display()
                        );
                        let panel = panel.clone();
                        crate::app::ui::primitives::tree_folder_row(
                            format!("review-folder-{}", path.display()),
                            label,
                            depth,
                            open,
                            true,
                            move |_, cx| {
                                let _ = panel.update(cx, |view, cx| {
                                    view.review_tree.toggle(&project, &path);
                                    cx.notify();
                                });
                            },
                        )
                        .aria_label(accessible)
                        .when(!open, |row| {
                            row.child(crate::app::ui::primitives::folder_change_summary(
                                count, counts,
                            ))
                        })
                        .into_any_element()
                    }
                    TreeRow::File { index, depth } => {
                        let location = &active.review.items[index];
                        // Preserve the selected range when clicking its file again.
                        let index = inspecting
                            .filter(|&selected| active.review.items[selected].path == location.path)
                            .unwrap_or(index);
                        let path = Path::new(&location.path);
                        div()
                            .pl(px(depth as f32 * 12.0))
                            .child(
                                location_button(
                                    "review-file",
                                    active,
                                    index,
                                    format!("Open {}", location.path),
                                    entity.clone(),
                                )
                                .w_full()
                                .min_w_0()
                                .h(px(24.0))
                                .px(THEME.space.xs)
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .rounded(THEME.radius)
                                .text_size(THEME.type_scale.caption)
                                .when(selected_path == Some(&location.path), |row| {
                                    row.bg(THEME.colors.selection)
                                })
                                .child(file_icon(path))
                                .child(
                                    div().min_w_0().flex_1().text_ellipsis().child(
                                        path.file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned(),
                                    ),
                                ),
                            )
                            .into_any_element()
                    }
                })),
        )
        .when_some(selected_path, |panel, path| {
            panel.child(
                div()
                    .id("review-selection")
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .text_size(THEME.type_scale.body_small)
                    .child(div().text_color(THEME.colors.muted).child(path.clone()))
                    .children(
                        active
                            .review
                            .items
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| &item.path == path)
                            .map(|(index, item)| {
                                let range = match (item.start_line, item.end_line) {
                                    (Some(start), Some(end)) => format!("Lines {start}–{end}"),
                                    (Some(start), None) => format!("Line {start}"),
                                    _ => "File".to_owned(),
                                };
                                let status = active
                                    .navigation
                                    .as_ref()
                                    .and_then(|nav| nav.locations.get(index));
                                location_button(
                                    "review-range",
                                    active,
                                    index,
                                    format!("{range}: {}", item.note),
                                    entity.clone(),
                                )
                                .p(THEME.space.xs)
                                .rounded(THEME.radius)
                                .when(inspecting == Some(index), |row| {
                                    row.bg(THEME.colors.selection)
                                })
                                .child(div().text_color(THEME.colors.muted).child(
                                    if selected == Some(index) {
                                        format!("{range} · Last opened here")
                                    } else {
                                        range
                                    },
                                ))
                                .child(item.note.clone())
                                .when_some(
                                    status.and_then(|status| status.warning.as_ref()),
                                    |row, warning| {
                                        row.child(
                                            div()
                                                .text_color(
                                                    if status.is_some_and(|status| status.valid) {
                                                        THEME.colors.muted
                                                    } else {
                                                        THEME.colors.warning
                                                    },
                                                )
                                                .child(warning.clone()),
                                        )
                                    },
                                )
                            }),
                    ),
            )
        })
        .into_any_element()
}

fn location_button(
    kind: &'static str,
    active: &ActiveReview,
    index: usize,
    label: String,
    entity: WeakEntity<FarcasterApp>,
) -> gpui::Stateful<gpui::Div> {
    let id = active.id;
    let available = active.pending.is_none() && active.navigation.is_some();
    let click = entity.clone();
    div()
        .id((kind, index))
        .role(gpui::Role::Button)
        .aria_label(label)
        .tab_index(0)
        .when(available, |row| {
            row.cursor_pointer().hover(|row| row.bg(THEME.colors.hover))
        })
        .focus_visible(|row| row.bg(THEME.colors.selection))
        .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
        .on_click(move |_, window, cx| {
            if available {
                let _ = click.update(cx, |this, cx| {
                    this.open_review_location(id, index, window, cx)
                });
            }
        })
        .on_key_down(move |event, window, cx| {
            if available && activates_button(event) {
                cx.stop_propagation();
                let _ = entity.update(cx, |this, cx| {
                    this.open_review_location(id, index, window, cx)
                });
            }
        })
}
