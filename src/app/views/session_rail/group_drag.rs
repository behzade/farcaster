use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Div, InteractiveElement as _, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
};

use super::{FarcasterApp, drag::DraggedSession};
use crate::app::ui::{
    primitives::{ReorderPosition, ReorderTargetExt as _},
    theme::theme,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum GroupTarget {
    Folder(u64),
    Project(PathBuf),
}

impl GroupTarget {
    fn can_reorder_to(&self, target: &Self) -> bool {
        self != target
            && matches!(
                (self, target),
                (Self::Folder(_), Self::Folder(_)) | (Self::Project(_), Self::Project(_))
            )
    }
}

#[derive(Clone)]
struct DraggedGroup {
    target: GroupTarget,
    label: String,
}

impl Render for DraggedGroup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .px(theme().space.sm)
            .py(theme().space.xs)
            .bg(theme().colors.surface)
            .text_color(theme().colors.muted)
            .rounded(theme().radius)
            .child(self.label.clone())
    }
}

pub(super) fn draggable_title(
    title: Stateful<Div>,
    target: GroupTarget,
    label: String,
    entity: WeakEntity<FarcasterApp>,
) -> Stateful<Div> {
    title.on_drag(DraggedGroup { target, label }, move |drag, _, _, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.sessions.group_drop_target = None;
            this.notify_session_rail(cx);
        });
        cx.new(|_| drag.clone())
    })
}

pub(super) fn group_drop_target(
    row: Stateful<Div>,
    target: GroupTarget,
    position: Option<ReorderPosition>,
    entity: WeakEntity<FarcasterApp>,
) -> Stateful<Div> {
    let accepts = target.clone();
    let hovered = target.clone();
    let moved = entity.clone();
    row.can_drop(move |value, _, _| {
        value
            .downcast_ref::<DraggedGroup>()
            .is_some_and(|drag| drag.target.can_reorder_to(&accepts))
            || (matches!(accepts, GroupTarget::Folder(_))
                && value
                    .downcast_ref::<DraggedSession>()
                    .is_some_and(|drag| drag.app_session_id > 0))
    })
    .reorder_target::<DraggedGroup>(
        position,
        theme().colors.indicator,
        theme().colors.highlight,
        move |position, _, cx| {
            let _ = moved.update(cx, |this, cx| {
                let next = Some((hovered.clone(), position));
                if this.sessions.group_drop_target != next {
                    this.sessions.group_drop_target = next;
                    this.notify_session_rail(cx);
                }
            });
        },
        move |drag, _, cx| {
            cx.stop_propagation();
            let _ = entity.update(cx, |this, cx| {
                let position = this
                    .sessions
                    .group_drop_target
                    .take()
                    .filter(|(group, _)| group == &target)
                    .map(|(_, position)| position);
                if let Some(position) = position {
                    this.reorder_session_group(&drag.target, &target, position, cx);
                }
                this.notify_session_rail(cx);
            });
        },
    )
}

impl FarcasterApp {
    pub(super) fn reorder_session_group(
        &mut self,
        source: &GroupTarget,
        target: &GroupTarget,
        position: ReorderPosition,
        cx: &mut Context<Self>,
    ) {
        if !source.can_reorder_to(target) {
            return;
        }
        let mut next = self.sessions.folders.clone();
        let after = position == ReorderPosition::After;
        let changed = match (source, target) {
            (GroupTarget::Folder(source), GroupTarget::Folder(target)) => {
                next.reorder_folder(*source, *target, after)
            }
            (GroupTarget::Project(source), GroupTarget::Project(target)) => {
                next.remember_projects(
                    self.sessions
                        .all
                        .iter()
                        .map(|session| session.project.clone())
                        .chain(
                            self.sessions
                                .drafts
                                .iter()
                                .map(|draft| draft.project.clone()),
                        ),
                );
                next.reorder_project(source, target, after)
            }
            _ => false,
        };
        if changed {
            self.save_session_folders(next, cx);
        }
    }
}

#[cfg(test)]
#[path = "group_drag_tests.rs"]
mod tests;
