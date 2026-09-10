use std::time::Duration;

use super::*;

const NATIVE_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::app) mod code_comment;
pub(in crate::app) mod code_tasks;
mod editor;
pub(in crate::app) mod neovim;
mod regions;
pub(in crate::app) mod review;
pub(in crate::app) mod runtime_picker;
mod surfaces;
mod terminal;
pub(in crate::app) mod worker_tasks;

pub(crate) use surfaces::{CycleWorkspaceBackward, CycleWorkspaceForward};

impl FarcasterApp {
    fn monitor_native_process(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        mut should_continue: impl FnMut(&mut Self, &mut Window, &mut Context<Self>) -> bool + 'static,
    ) {
        cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(NATIVE_PROCESS_POLL_INTERVAL)
                    .await;
                let keep_polling = weak
                    .update_in(cx, |this, window, cx| should_continue(this, window, cx))
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }
}
