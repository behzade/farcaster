use std::path::{Path, PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    projects::{self, StartupTrust, TrustChoice},
    runtime::RuntimeCommand,
};

fn trust_path() -> Result<PathBuf, String> {
    Ok(crate::app::paths::data_dir()?.join("project-trust.json"))
}

pub(in crate::app) fn startup_trust(project: &Path) -> Result<StartupTrust, String> {
    projects::startup_trust(&trust_path()?, project)
}

pub(in crate::app) fn repository_execution_allowed(project: &Path) -> Result<bool, String> {
    projects::repository_execution_allowed(&trust_path()?, project)
}

pub(in crate::app) fn saved_decision(project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
    projects::saved_decision(&trust_path()?, project)
}

pub(in crate::app) fn apply(
    project: &Path,
    choice: TrustChoice,
) -> Result<projects::AppliedTrust, String> {
    projects::apply(&trust_path()?, project, choice)
}

impl FarcasterApp {
    pub(in crate::app) fn send_project_command(
        &mut self,
        project: &Path,
        command: RuntimeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_project_trust_command.is_some() {
            return;
        }
        match startup_trust(project) {
            Ok(StartupTrust::Ready) => {
                let backend = command_backend(&command)
                    .unwrap_or(&self.snapshot.harness)
                    .to_owned();
                if self.ensure_backend_trust(&backend, project, window, cx) {
                    self.send(command, cx);
                } else {
                    self.pending_project_trust_command = Some(command);
                }
            }
            result => {
                self.open_project_trust(window, cx);
                self.project_trust_project = Some(project.to_path_buf());
                self.project_trust_error = result.err();
                self.pending_project_trust_command = Some(command);
            }
        }
    }

    pub(in crate::app) fn save_project_trust(
        &mut self,
        choice: TrustChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self
            .project_trust_project
            .clone()
            .unwrap_or_else(|| self.project.clone());
        let backend = self.project_trust_backend.clone();
        let applied = match backend.as_deref() {
            Some(backend) => crate::agents::apply_project_trust(backend, &project, choice),
            None => apply(&project, choice),
        };
        match applied {
            Ok(applied) => {
                if backend.is_none() {
                    self.set_repository_project_execution(project.clone(), applied.trusted, cx);
                }
                let scope = applied.saved_path.map_or_else(
                    || self.project.display().to_string(),
                    |path| path.display().to_string(),
                );
                self.project_trust_error = None;
                self.project_trust_project = None;
                self.project_trust_backend = None;
                let pending = self
                    .pending_project_trust_command
                    .take()
                    .map(restart_session_after_trust);
                self.close_sheet(window, cx);
                if let Some(command) = pending {
                    self.send_project_command(&project, command, window, cx);
                } else {
                    let decision = if applied.trusted {
                        "trusted"
                    } else {
                        "untrusted"
                    };
                    let status = match backend {
                        Some(backend) => format!(
                            "{} project {decision} in {scope}. Restart existing sessions to apply the new decision.",
                            crate::agents::backend_display_name(&backend)
                        ),
                        None => format!("Farcaster project {decision} in {scope}."),
                    };
                    std::sync::Arc::make_mut(&mut self.snapshot).status = status;
                    self.notify_composer(cx);
                }
            }
            Err(error) => {
                self.project_trust_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(in crate::app) fn dismiss_project_trust(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cancel_pending_command(&mut self.pending_project_trust_command)
            && let Some((_, timing)) = self.pending_session_switch.take()
        {
            timing.cancel();
        }
        self.project_trust_error = None;
        self.project_trust_project = None;
        self.project_trust_backend = None;
        self.close_sheet(window, cx);
    }

    pub(in crate::app) fn ensure_backend_trust(
        &mut self,
        backend: &str,
        project: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match crate::agents::project_trust(backend, project) {
            Ok(StartupTrust::Ready) => true,
            result => {
                self.open_backend_project_trust(
                    backend.to_owned(),
                    project.to_path_buf(),
                    window,
                    cx,
                );
                self.project_trust_error = result.err();
                false
            }
        }
    }

    pub(in crate::app) fn open_backend_project_trust(
        &mut self,
        backend: String,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_project_trust(window, cx);
        self.project_trust_project = Some(project);
        self.project_trust_backend = Some(backend);
    }
}

fn command_backend(command: &RuntimeCommand) -> Option<&str> {
    match command {
        RuntimeCommand::NewSession { harness, .. }
        | RuntimeCommand::ResumeDraft { harness, .. }
        | RuntimeCommand::SelectSession { harness, .. }
        | RuntimeCommand::RestartSession { harness, .. }
        | RuntimeCommand::ForkSession { harness, .. } => Some(harness),
        _ => None,
    }
}

fn cancel_pending_command(pending: &mut Option<RuntimeCommand>) -> bool {
    pending.take().is_some()
}

fn restart_session_after_trust(command: RuntimeCommand) -> RuntimeCommand {
    match command {
        RuntimeCommand::SelectSession {
            path,
            harness,
            session_id,
            project,
        } => RuntimeCommand::RestartSession {
            path,
            harness,
            session_id,
            project,
        },
        command => command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissing_trust_cancels_the_pending_project_command() {
        let mut pending = Some(RuntimeCommand::Shutdown);
        assert!(cancel_pending_command(&mut pending));
        assert!(pending.is_none());
    }

    #[test]
    fn selecting_after_a_new_trust_decision_restarts_the_project_process() {
        let path = PathBuf::from("/session.jsonl");
        let project = PathBuf::from("/project");
        assert!(matches!(
            restart_session_after_trust(RuntimeCommand::SelectSession {
                session_id: path.to_string_lossy().into_owned(),
                path,
                harness: "pi".into(),
                project,
            }),
            RuntimeCommand::RestartSession { .. }
        ));
    }
}
