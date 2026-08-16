//! File-change modal lifecycle and focus restoration.

use gpui::{Context, Window};

use super::PiApp;
use crate::conversation::ToolPresentation;

impl PiApp {
    pub(crate) fn open_change_modal(
        &mut self,
        presentation: ToolPresentation,
        tool_call_id: Option<String>,
        key: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.change_modal.is_none() {
            self.dialog_return_focus = window.focused(cx);
        }
        self.change_modal = Some(crate::tool_changes::ChangeModal {
            presentation,
            tool_call_id,
            key,
        });
        self.pending_dialog_setup = true;
        cx.notify();
    }

    pub(crate) fn close_change_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.change_modal.take().is_none() {
            return;
        }
        self.pending_dialog_setup = false;
        self.restore_dialog_focus(window, cx);
    }
}
