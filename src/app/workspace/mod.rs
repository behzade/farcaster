use std::time::Duration;

use gpui_libghostty::{TerminalConfiguration, TerminalOptions};

use super::*;

mod app_state;
pub(in crate::app) use app_state::{EditorState, SettingsState, TerminalState, WorkspaceState};

mod covered_refresh;

const NATIVE_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::app) mod code_tasks;
pub(in crate::app) mod editor;
pub(in crate::app) mod editor_session;
mod harness_profiles;
mod regions;
pub(in crate::app) mod review;
pub(in crate::app) mod runtime_picker;
pub(in crate::app) mod send_to_chat;
mod surfaces;
mod terminal;
pub(in crate::app) mod theme_settings;
pub(in crate::app) use terminal::Terminal;
pub(in crate::app) mod worker_tasks;

pub(crate) use surfaces::{CycleWorkspaceBackward, CycleWorkspaceForward};

pub(in crate::app) fn spawn_workspace_terminal<T: 'static>(
    command: String,
    project: PathBuf,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Result<Entity<Terminal>, String> {
    let mut options = TerminalOptions::new(command, project);
    options.configuration = TerminalConfiguration::Custom(crate::app::ui::theme::terminal_theme());
    Terminal::spawn(options, window, cx)
}

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
