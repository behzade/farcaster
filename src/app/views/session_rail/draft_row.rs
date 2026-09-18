use std::time::{Duration, UNIX_EPOCH};

use gpui::{
    App, AppContext as _, CursorStyle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div, prelude::FluentBuilder as _,
};

use super::{
    drag::DraggedSession,
    groups::SessionRailKind,
    hover::{draft_hover_details, session_tooltip_content},
    rows::{project_badge, project_label, relative_age, session_row_metadata},
};
use crate::{
    app::FarcasterApp,
    app::ui::primitives::{AppTooltip as _, DeleteButton, ReorderPosition, ReorderTargetExt as _},
    app::ui::theme::theme,
    sessions::DraftSession,
};

pub(super) struct DraftRowInput {
    pub(super) selected: bool,
    pub(super) status: String,
    pub(super) drop_position: Option<ReorderPosition>,
    pub(super) nested: bool,
}

#[derive(IntoElement)]
pub(super) struct DraftRow {
    draft: DraftSession,
    input: DraftRowInput,
    entity: WeakEntity<FarcasterApp>,
}

impl DraftRow {
    pub(super) fn new(
        draft: &DraftSession,
        input: DraftRowInput,
        entity: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            draft: draft.clone(),
            input,
            entity,
        }
    }
}

impl RenderOnce for DraftRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            draft,
            input:
                DraftRowInput {
                    selected,
                    status,
                    drop_position,
                    nested,
                },
            entity,
        } = self;
        let status = status.as_str();
        let is_draft = status == "Draft";
        let age = relative_age(UNIX_EPOCH + Duration::from_millis(draft.created_ms));
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
        let hover_details = draft_hover_details(&draft, status);
        div()
            .h(theme().layout.session_row_height)
            .w_full()
            .child(
                div()
                    .id(format!("session-{id}"))
                    .app_tooltip_element(move |_, _| session_tooltip_content(&hover_details))
                    .role(Role::Button)
                    .aria_label(format!("Open {status} session in {}", project.display()))
                    .aria_selected(selected)
                    .tab_index(0)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        crate::app::ui::primitives::preserve_pointer_focus,
                    )
                    .size_full()
                    .h(theme().layout.session_row_height)
                    .relative()
                    .flex()
                    .items_stretch()
                    .px(theme().space.sm)
                    .when(nested, |row| row.pl(theme().space.md))
                    .py(theme().space.xs)
                    .rounded(theme().radius)
                    .group(action_group.clone())
                    .bg(if selected {
                        theme().colors.highlight
                    } else {
                        theme().colors.panel
                    })
                    .hover(|row| row.bg(theme().colors.highlight))
                    .when(selected, |row| {
                        row.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(theme().size(2.0))
                                .bg(theme().colors.indicator),
                        )
                    })
                    .focus(|row| {
                        row.border(theme().border)
                            .border_color(theme().colors.indicator)
                    })
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
                        theme().colors.indicator,
                        theme().colors.highlight,
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
                            this.resume_draft_and_focus(id.clone(), project.clone(), window, cx);
                        });
                    })
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(theme().space.sm)
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(theme().type_scale.body_small)
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(theme().colors.text)
                                    .child(title),
                            )
                            .when(!nested, |row| {
                                row.child(
                                    div()
                                        .max_w(theme().size(120.0))
                                        .flex_none()
                                        .child(project_badge(&draft.project)),
                                )
                            })
                            .child(
                                DeleteButton::new(format!("discard-{discard_id}"), "Discard draft")
                                    .reveal_on(action_group)
                                    .on_delete(move |window, cx| {
                                        let _ = discard_entity.update(cx, |this, cx| {
                                            this.discard_draft(&discard_id, window, cx);
                                        });
                                    }),
                            )
                            .when_some(
                                super::rows::failure_indicator(target_app_session_id, status),
                                |row, indicator| row.child(indicator),
                            )
                            .child(session_row_metadata(
                                draft.harness,
                                target_app_session_id,
                                if is_draft { "" } else { status },
                                age,
                            )),
                    ),
            )
            .into_any_element()
    }
}
