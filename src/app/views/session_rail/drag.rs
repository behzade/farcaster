use std::path::PathBuf;

use gpui::{
    Context, FontWeight, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};

use super::groups::SessionRailKind;
use crate::app::ui::theme::THEME;

#[derive(Clone)]
pub(super) struct DraggedSession {
    pub(super) app_session_id: i64,
    pub(super) path: Option<PathBuf>,
    pub(super) kind: SessionRailKind,
    pub(super) title: String,
    pub(super) project: String,
}

impl DraggedSession {
    pub(super) fn can_move_to(&self, kind: SessionRailKind) -> bool {
        self.path.is_some() && self.kind != kind
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
            .w(px(260.0))
            .px(THEME.space.md)
            .py(THEME.space.sm)
            .rounded(THEME.radius)
            .bg(THEME.colors.surface)
            .border(THEME.border)
            .border_color(THEME.colors.accent)
            .shadow_md()
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.text)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(self.project.clone()),
            )
    }
}
