use gpui::{
    Context, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, Window, div,
};

use super::groups::SessionRailKind;
use crate::app::ui::theme::theme;

#[derive(Clone)]
pub(super) struct DraggedSession {
    pub(super) app_session_id: i64,
    pub(super) kind: SessionRailKind,
    pub(super) title: String,
    pub(super) project: String,
}

impl DraggedSession {
    /// Every chat can be filed away and brought back, including one that has
    /// not been written to a session yet, so this only needs an identity.
    pub(super) fn can_move_to(&self, kind: SessionRailKind) -> bool {
        self.app_session_id > 0 && self.kind != kind
    }

    pub(super) fn can_drop_on(&self, kind: SessionRailKind, target: i64) -> bool {
        self.can_move_to(kind)
            || (kind == SessionRailKind::Project
                && self.kind == kind
                && self.app_session_id > 0
                && target > 0
                && self.app_session_id != target)
    }
}

impl Render for DraggedSession {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(theme().size(260.0))
            .px(theme().space.md)
            .py(theme().space.sm)
            .rounded(theme().radius)
            .bg(theme().colors.surface)
            .border(theme().border)
            .border_color(theme().colors.indicator)
            .shadow_md()
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme().colors.text)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mt(theme().size(2.0))
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.subtle)
                    .child(self.project.clone()),
            )
    }
}
