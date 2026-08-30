use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    project_trust::{self, StartupTrust, TrustChoice},
    runtime::RuntimeCommand,
};

impl FarcasterApp {
    pub(super) fn send_project_command(
        &mut self,
        project: &std::path::Path,
        command: RuntimeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        match project_trust::startup_trust(project) {
            Ok(StartupTrust::Ready) => self.send(command),
            Ok(StartupTrust::Prompt) => {
                self.open_project_trust(window, cx);
                self.project_trust_project = Some(project.to_path_buf());
                self.pending_project_trust_command = Some(command);
            }
            Err(error) => {
                self.open_project_trust(window, cx);
                self.project_trust_project = Some(project.to_path_buf());
                self.project_trust_error = Some(error);
                self.pending_project_trust_command = Some(command);
            }
        }
    }

    pub(super) fn save_project_trust(
        &mut self,
        choice: TrustChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self
            .project_trust_project
            .clone()
            .unwrap_or_else(|| self.project.clone());
        match project_trust::apply(&project, choice) {
            Ok(applied) => {
                self.set_repository_project_execution(project, applied.trusted, cx);
                let scope = applied.saved_path.map_or_else(
                    || self.project.display().to_string(),
                    |path| path.display().to_string(),
                );
                self.project_trust_error = None;
                self.project_trust_project = None;
                let pending = self
                    .pending_project_trust_command
                    .take()
                    .map(restart_session_after_trust);
                self.close_sheet(window, cx);
                if let Some(command) = pending {
                    self.send(command);
                } else {
                    let decision = if applied.trusted {
                        "trusted"
                    } else {
                        "untrusted"
                    };
                    std::sync::Arc::make_mut(&mut self.snapshot).status = format!(
                        "Project {decision} in {scope}. Restart Farcaster to apply the new decision."
                    );
                    self.notify_composer(cx);
                }
            }
            Err(error) => {
                self.project_trust_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn dismiss_project_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        self.project_trust_project = None;
        self.close_sheet(window, cx);
    }
}

fn restart_session_after_trust(command: RuntimeCommand) -> RuntimeCommand {
    match command {
        RuntimeCommand::SelectSession { path, project } => {
            RuntimeCommand::RestartSession { path, project }
        }
        command => command,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn selecting_after_a_new_trust_decision_restarts_the_project_process() {
        let path = PathBuf::from("/session.jsonl");
        let project = PathBuf::from("/project");
        assert!(matches!(
            restart_session_after_trust(RuntimeCommand::SelectSession { path, project }),
            RuntimeCommand::RestartSession { .. }
        ));
    }
}
