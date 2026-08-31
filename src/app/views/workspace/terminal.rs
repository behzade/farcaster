use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div};

use crate::app::FarcasterApp;
use crate::app::ui::theme::THEME;

impl FarcasterApp {
    pub(in crate::app::views) fn render_terminal_workspace(&self) -> AnyElement {
        if let Some(error) = self.terminal_error.clone() {
            return terminal_error(error);
        }
        if let Some(terminal) = self.terminal.clone() {
            return div()
                .size_full()
                .min_h_0()
                .child(terminal)
                .into_any_element();
        }
        terminal_error("Terminal is not running".to_owned())
    }
}

fn terminal_error(message: String) -> AnyElement {
    div()
        .size_full()
        .p(THEME.space.md)
        .text_color(THEME.colors.error)
        .child(message)
        .into_any_element()
}
