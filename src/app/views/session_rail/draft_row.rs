use gpui::{
    AnyElement, AppContext as _, CursorStyle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::kbd::Kbd;

use super::{
    drag::DraggedSession,
    groups::SessionRailKind,
    hover::{draft_hover_details, session_hover_panel},
    rows::{project_badge, project_label, status_icon},
};
use crate::{
    app::FarcasterApp,
    app::ui::assets::AppIcon,
    app::ui::keybindings::application_modifier,
    app::ui::primitives::{
        AppIconSize, ReorderPosition, ReorderTargetExt as _, app_icon, icon_control,
    },
    app::ui::theme::THEME,
    projects::DraftSession,
};

pub(super) fn draft_session_row(
    draft: &DraftSession,
    selected: bool,
    status: &str,
    shortcut: Option<u8>,
    drop_position: Option<ReorderPosition>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let id = draft.id.clone();
    let discard_id = id.clone();
    let project = draft.project.clone();
    let discard_entity = entity.clone();
    let title = draft.title.as_deref().unwrap_or("New session").to_owned();
    let target_app_session_id = draft.app_session_id;
    let drag = DraggedSession {
        app_session_id: target_app_session_id,
        path: draft.session_path.clone(),
        kind: SessionRailKind::Project,
        title: title.clone(),
        project: project_label(&draft.project),
    };
    let drag_move_entity = entity.clone();
    let drop_entity = entity.clone();
    let drag_entity = entity.clone();
    let action_group = format!("draft-actions-{id}");
    let hover_id = format!("draft-hover-{id}");
    let hover_details = draft_hover_details(draft, status);
    session_hover_panel(
        hover_id,
        hover_details,
        div()
            .h(THEME.layout.session_row_height)
            .w_full()
            .px(px(2.0))
            .child(
                div()
                    .id(format!("session-{id}"))
                    .role(Role::Button)
                    .aria_label(format!("Open {status} session in {}", project.display()))
                    .aria_selected(selected)
                    .tab_index(0)
                    .size_full()
                    .h(THEME.layout.session_row_height)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .px(THEME.space.sm)
                    .rounded(px(2.0))
                    .group(action_group.clone())
                    .bg(if selected {
                        THEME.colors.selection
                    } else {
                        THEME.colors.panel
                    })
                    .hover(|row| row.bg(THEME.colors.surface))
                    .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                    .cursor(CursorStyle::PointingHand)
                    .on_drag(drag, move |drag, _, _, cx| {
                        let _ = drag_entity.update(cx, |this, cx| this.begin_session_drag(cx));
                        cx.new(|_| drag.clone())
                    })
                    .can_drop(move |value, _, _| {
                        value.downcast_ref::<DraggedSession>().is_some_and(|drag| {
                            drag.can_drop_on(SessionRailKind::Project, target_app_session_id)
                        })
                    })
                    .reorder_target::<DraggedSession>(
                        drop_position,
                        THEME.colors.accent,
                        THEME.colors.hover,
                        move |position, _, cx| {
                            let _ = drag_move_entity.update(cx, |this, cx| {
                                this.update_session_drop_target(
                                    target_app_session_id,
                                    position,
                                    cx,
                                );
                            });
                        },
                        move |drag, window, cx| {
                            cx.stop_propagation();
                            let _ = drop_entity.update(cx, |this, cx| {
                                this.complete_session_row_drop(
                                    drag,
                                    SessionRailKind::Project,
                                    window,
                                    cx,
                                );
                            });
                        },
                    )
                    .on_click(move |_, window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.resume_draft(id.clone(), project.clone(), window, cx);
                        });
                    })
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .pr(px(24.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(THEME.type_scale.body_small)
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(THEME.colors.text)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(THEME.space.xs)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .child(project_badge(&draft.project)),
                                    )
                                    .when(shows_draft_badge(status), |metadata| {
                                        metadata.child(draft_badge())
                                    })
                                    .child(app_icon(
                                        AppIcon::for_harness(&draft.harness),
                                        AppIconSize::Inline,
                                    ))
                                    .when(status != "Draft", |metadata| {
                                        metadata.when_some(
                                            status_icon(target_app_session_id, status),
                                            |metadata, icon| metadata.child(icon),
                                        )
                                    })
                                    .when_some(shortcut, |metadata, number| {
                                        metadata.child(Kbd::new(
                                            gpui::Keystroke::parse(&format!(
                                                "{}-{number}",
                                                application_modifier().prefix()
                                            ))
                                            .expect("fixed session shortcut must parse"),
                                        ))
                                    })
                                    .when(
                                        draft_can_be_discarded(draft.submitted, status),
                                        |metadata| {
                                            metadata.child(
                                                icon_control(
                                                    format!("discard-{discard_id}"),
                                                    "Discard draft",
                                                )
                                                .absolute()
                                                .top(px(4.0))
                                                .right(px(5.0))
                                                .size(px(21.0))
                                                .opacity(0.0)
                                                .group_hover(action_group, |button| {
                                                    button.opacity(1.0)
                                                })
                                                .focus(|button| button.opacity(1.0))
                                                .hover(|button| button.bg(THEME.colors.hover))
                                                .child(app_icon(
                                                    AppIcon::Trash,
                                                    AppIconSize::Control,
                                                ))
                                                .on_click(move |_, window, cx| {
                                                    cx.stop_propagation();
                                                    let _ =
                                                        discard_entity.update(cx, |this, cx| {
                                                            this.discard_draft(
                                                                &discard_id,
                                                                window,
                                                                cx,
                                                            );
                                                        });
                                                }),
                                            )
                                        },
                                    ),
                            ),
                    ),
            )
            .into_any_element(),
    )
}

fn draft_can_be_discarded(submitted: bool, status: &str) -> bool {
    !submitted || status == "Failed"
}

fn draft_badge() -> AnyElement {
    div()
        .flex_none()
        .px(THEME.space.xs)
        .rounded(px(3.0))
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.muted)
        .child("Draft")
        .into_any_element()
}

fn shows_draft_badge(status: &str) -> bool {
    status == "Draft"
}

#[cfg(test)]
mod tests {
    use super::{draft_can_be_discarded, shows_draft_badge};

    #[test]
    fn submitted_session_status_replaces_the_draft_badge() {
        assert!(shows_draft_badge("Draft"));
        assert!(!shows_draft_badge("Working"));
        assert!(!shows_draft_badge("Needs input"));
        assert!(!shows_draft_badge("Done"));
    }

    #[test]
    fn failed_submitted_drafts_can_be_discarded() {
        assert!(draft_can_be_discarded(true, "Failed"));
        assert!(!draft_can_be_discarded(true, "Working"));
        assert!(draft_can_be_discarded(false, "Draft"));
    }
}
