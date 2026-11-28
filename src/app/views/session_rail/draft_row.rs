use crate::app::ui::primitives::SelectionStyle as _;
use crate::{app::session::status::run_status_label, runtime::RunStatus};
use gpui::{
    AnyElement, App, AppContext as _, CursorStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div,
};

use super::{
    drag::DraggedSession,
    groups::SessionRailKind,
    hover::{draft_hover_details, session_tooltip_content},
    rows::{
        archive_action, project_badge, project_label, session_row_content, session_row_height,
        session_row_trailing, session_status_icon, session_title,
    },
};
use crate::{
    app::FarcasterApp,
    app::ui::primitives::{AppTooltip as _, DeleteButton, ReorderPosition, ReorderTargetExt as _},
    app::ui::theme::theme,
    sessions::DraftSession,
};

fn archive_draft_action(
    id: &str,
    archived: bool,
    action_group: String,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let draft_id = id.to_owned();
    archive_action(id, archived, action_group, move |apply, window, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.request_draft_archive(draft_id.clone(), apply, window, cx);
        });
    })
}

pub(super) struct DraftRowInput {
    pub(super) selected: bool,
    pub(super) status: RunStatus,
    pub(super) archived: bool,
    pub(super) drop_position: Option<ReorderPosition>,
    pub(super) compact: bool,
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
                    archived,
                    drop_position,
                    compact,
                },
            entity,
        } = self;
        let compact = compact && !archived;
        let is_draft = status == RunStatus::Draft;
        let status_text = run_status_label(status);
        let id = draft.id.clone();
        let identity = format!("draft:{id}");
        let discard_id = id.clone();
        let project = draft.project.clone();
        let discard_entity = entity.clone();
        let archive_entity = entity.clone();
        let archive_id = id.clone();
        let title = draft.title.as_deref().unwrap_or("New session").to_owned();
        let target_app_session_id = draft.app_session_id;
        let drag = DraggedSession {
            app_session_id: target_app_session_id,
            kind: if archived {
                SessionRailKind::Archived
            } else {
                SessionRailKind::Project
            },
            title: title.clone(),
            project: project_label(&draft.project),
        };
        let drag_move_entity = entity.clone();
        let drop_entity = entity.clone();
        let drag_entity = entity.clone();
        let action_group = format!("draft-actions-{id}");
        let hover_details = draft_hover_details(&draft, status_text);
        div()
            .h(session_row_height(compact))
            .w_full()
            .child(
                div()
                    .id(format!("session-{id}"))
                    .app_tooltip_element(move |_, _| session_tooltip_content(&hover_details))
                    .role(Role::Button)
                    .aria_label(format!(
                        "Open {status_text} session in {}",
                        project.display()
                    ))
                    .aria_selected(selected)
                    .tab_index(0)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        crate::app::ui::primitives::preserve_pointer_focus,
                    )
                    .size_full()
                    .h(session_row_height(compact))
                    .relative()
                    .flex()
                    .items_stretch()
                    .px(super::rail_inset())
                    .rounded(theme().radius)
                    .group(action_group.clone())
                    .bg(theme().colors.panel)
                    .selection_style(selected)
                    .hover(|row| row.bg(theme().colors.highlight))
                    .focus(|row| {
                        row.shadow(crate::app::ui::primitives::focus_outline(
                            theme().colors.indicator,
                        ))
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
                    .child(session_row_content(
                        compact,
                        session_title(title, selected, archived).into_any_element(),
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .child(project_badge(&draft.project))
                            .into_any_element(),
                        session_row_trailing(
                            draft.harness,
                            session_status_icon(&identity, (!is_draft).then_some(status)),
                            (draft.submitted || archived).then(|| {
                                archive_draft_action(
                                    &archive_id,
                                    archived,
                                    action_group.clone(),
                                    archive_entity,
                                )
                            }),
                            (!draft.submitted || archived).then(|| {
                                DeleteButton::new(format!("discard-{discard_id}"), "Discard draft")
                                    .reveal_on(action_group.clone())
                                    .on_delete(move |window, cx| {
                                        let _ = discard_entity.update(cx, |this, cx| {
                                            this.discard_draft(&discard_id, window, cx);
                                        });
                                    })
                                    .into_any_element()
                            }),
                            action_group.clone(),
                        ),
                    )),
            )
            .into_any_element()
    }
}
