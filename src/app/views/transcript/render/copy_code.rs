use std::time::Duration;

use gpui::{App, ClipboardItem, IntoElement, RenderOnce, SharedString, Task, Window};

use crate::app::ui::{
    assets::AppIcon,
    primitives::{ButtonTone, icon_button},
};

#[derive(Default)]
struct CopyState {
    copied_code: Option<SharedString>,
    reset_task: Option<Task<()>>,
}

#[derive(IntoElement)]
pub(super) struct CopyCodeButton {
    code: SharedString,
}

impl CopyCodeButton {
    pub(super) fn new(code: SharedString) -> Self {
        Self { code }
    }
}

impl RenderOnce for CopyCodeButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Render inside the block's element namespace so each button owns its state.
        let state = window.use_keyed_state("copy-code-state", cx, |_, _| CopyState::default());
        let copied = state.read(cx).copied_code.as_ref() == Some(&self.code);
        icon_button(
            "copy-code",
            if copied {
                AppIcon::CheckCircle
            } else {
                AppIcon::Copy
            },
            "Copy code",
            ButtonTone::Quiet,
            move |_, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(self.code.to_string()));
                state.update(cx, |state, cx| {
                    state.copied_code = Some(self.code.clone());
                    // Replacing the task restarts the delay on repeated clicks.
                    state.reset_task = Some(cx.spawn(async move |state, cx| {
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        let _ = state.update(cx, |state, cx| {
                            state.copied_code = None;
                            state.reset_task = None;
                            cx.notify();
                        });
                    }));
                    cx.notify();
                });
            },
        )
    }
}
