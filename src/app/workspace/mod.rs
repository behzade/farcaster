//! Native workspace surfaces and focus coordination.

use std::time::Duration;

use super::*;

const NATIVE_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

mod editor;
mod regions;
mod surfaces;
mod terminal;

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
