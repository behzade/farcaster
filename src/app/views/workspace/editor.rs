use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div};

use crate::app::FarcasterApp;

impl FarcasterApp {
    pub(in crate::app::views) fn render_editor_surface(&self) -> AnyElement {
        div()
            .size_full()
            .min_h_0()
            .children(self.workspace.editor.view.clone())
            .children(self.workspace.editor.terminal_editor_view.clone())
            .into_any_element()
    }
}
