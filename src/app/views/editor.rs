use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div};

use super::super::FarcasterApp;
use crate::app::ui::theme::THEME;

impl FarcasterApp {
    pub(super) fn render_editor_surface(&self) -> AnyElement {
        let content = if let Some(error) = self.editor_error.clone() {
            div()
                .size_full()
                .p(THEME.space.md)
                .text_color(THEME.colors.error)
                .child(error)
                .into_any_element()
        } else if let Some(editor) = self.editor.clone() {
            editor.into_any_element()
        } else {
            div()
                .size_full()
                .p(THEME.space.md)
                .text_color(THEME.colors.error)
                .child("Neovim is not running")
                .into_any_element()
        };
        div()
            .size_full()
            .min_h_0()
            .child(content)
            .into_any_element()
    }
}
