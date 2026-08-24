//! Embedded Neovim editor surface.

use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div};

use super::super::PiApp;
use crate::theme::THEME;

impl PiApp {
    pub(super) fn render_editor_surface(&self) -> AnyElement {
        let content = if let Some(editor) = self.editor.clone() {
            editor.into_any_element()
        } else {
            div()
                .size_full()
                .p(THEME.space.md)
                .text_color(THEME.colors.error)
                .child(
                    self.editor_error
                        .clone()
                        .unwrap_or_else(|| "Neovim is not running".to_owned()),
                )
                .into_any_element()
        };
        div()
            .size_full()
            .min_h_0()
            .child(content)
            .into_any_element()
    }
}
