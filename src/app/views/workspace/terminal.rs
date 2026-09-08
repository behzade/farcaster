use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div};

use crate::app::FarcasterApp;

impl FarcasterApp {
    pub(in crate::app::views) fn render_terminal_workspace(&self) -> AnyElement {
        div()
            .size_full()
            .min_h_0()
            .children(self.terminal.clone())
            .into_any_element()
    }
}
