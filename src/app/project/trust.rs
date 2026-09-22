use crate::agents::Backend;
use std::path::{Path, PathBuf};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::{
    app::workspace::send_to_chat::CodeDestination,
    projects::{self, StartupTrust, TrustChoice},
    runtime::{RuntimeCommand, TaskSettings},
};

pub(in crate::app) enum PendingTrustAction {
    Terminal(PathBuf),
    SendToChat {
        destination: CodeDestination,
        project: PathBuf,
        message: String,
    },
    StartCodeTask {
        settings: TaskSettings,
        message: String,
    },
}

impl PendingTrustAction {
    fn project(&self) -> &Path {
        match self {
            Self::Terminal(project) | Self::SendToChat { project, .. } => project,
            Self::StartCodeTask { settings, .. } => &settings.project,
        }
    }
}

fn take_trust_action(
    pending: &mut Option<PendingTrustAction>,
    trusted: bool,
    project: &Path,
) -> Option<PendingTrustAction> {
    let action = pending.take()?;
    (trusted && action.project() == project).then_some(action)
}

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
    pub(in crate::app) fn request_project_trust_for_action(
        &mut self,
        project: PathBuf,
        action: PendingTrustAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_project_trust(window, cx);
        self.project.trust_project = Some(project);
        self.project.pending_trust_action = Some(action);
    }

    fn resume_trust_action(
        &mut self,
        action: PendingTrustAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_project = self.workspace_project();
        if matches!(&action, PendingTrustAction::Terminal(_))
            && action.project() != current_project.as_path()
        {
            return;
        }
        match action {
            PendingTrustAction::Terminal(project) => {
                self.activate_terminal_for_project(project, window, cx);
            }
            PendingTrustAction::SendToChat {
                destination,
                project,
                message,
            } => {
                self.submit_to_chat(destination, project, message, window, cx);
            }
            PendingTrustAction::StartCodeTask { settings, message } => {
                self.submit_code_task(settings, message, window, cx);
            }
        }
    }

    pub(in crate::app) fn send_project_command(
        &mut self,
        project: &Path,
        command: RuntimeCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.pending_trust_command.is_some() {
            return;
        }
        match startup_trust(project) {
            Ok(StartupTrust::Ready) => {
                let backend = command_backend(&command).or(self.snapshot.harness);
                if backend
                    .is_none_or(|backend| self.ensure_backend_trust(backend, project, window, cx))
                {
                    self.send(command, cx);
                } else {
                    self.project.pending_trust_command = Some(command);
                }
            }
            result => {
                self.open_project_trust(window, cx);
                self.project.trust_project = Some(project.to_path_buf());
                self.project.trust_error = result.err();
                self.project.pending_trust_command = Some(command);
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
            .project
            .trust_project
            .clone()
            .unwrap_or_else(|| self.project.path.clone());
        let backend = self.project.trust_backend;
        let applied = match backend {
            Some(backend) => crate::agents::apply_project_trust(backend, &project, choice),
            None => apply(&project, choice),
        };
        match applied {
            Ok(applied) => {
                if backend.is_none() {
                    self.set_repository_project_execution(project.clone(), applied.trusted, cx);
                }
                let scope = applied.saved_path.map_or_else(
                    || project.display().to_string(),
                    |path| path.display().to_string(),
                );
                self.project.trust_error = None;
                self.project.trust_project = None;
                self.project.trust_backend = None;
                let pending =
                    take_trust_command(&mut self.project.pending_trust_command, applied.trusted);
                let pending_action = take_trust_action(
                    &mut self.project.pending_trust_action,
                    applied.trusted,
                    &project,
                );
                self.close_sheet(window, cx);
                if let Some(command) = pending {
                    self.send_project_command(&project, command, window, cx);
                } else if let Some(action) = pending_action {
                    self.resume_trust_action(action, window, cx);
                } else {
                    let decision = if applied.trusted {
                        "trusted"
                    } else {
                        "untrusted"
                    };
                    let status = match backend {
                        Some(backend) => format!(
                            "{} project {decision} in {scope}. Restart existing sessions to apply the new decision.",
                            crate::agents::backend_display_name(backend)
                        ),
                        None => format!("Farcaster project {decision} in {scope}."),
                    };
                    std::sync::Arc::make_mut(&mut self.snapshot).status = status;
                    self.notify_composer(cx);
                }
            }
            Err(error) => {
                self.project.trust_error = Some(error);
                cx.notify();
            }
        }
    }

    pub(in crate::app) fn dismiss_project_trust(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cancel_pending_command(&mut self.project.pending_trust_command)
            && let Some((_, timing)) = self.lifecycle.pending_session_switch.take()
        {
            timing.cancel();
        }
        self.project.trust_error = None;
        self.project.trust_project = None;
        self.project.trust_backend = None;
        self.project.pending_trust_action = None;
        self.close_sheet(window, cx);
    }

    pub(in crate::app) fn ensure_backend_trust(
        &mut self,
        backend: Backend,
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
                self.project.trust_error = result.err();
                false
            }
        }
    }

    pub(in crate::app) fn open_backend_project_trust(
        &mut self,
        backend: Backend,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_project_trust(window, cx);
        self.project.trust_project = Some(project);
        self.project.trust_backend = Some(backend);
    }
}

fn command_backend(command: &RuntimeCommand) -> Option<Backend> {
    match command {
        RuntimeCommand::NewSession { harness, .. }
        | RuntimeCommand::ResumeDraft { harness, .. } => *harness,
        RuntimeCommand::SelectSession { harness, .. }
        | RuntimeCommand::RestartSession { harness, .. }
        | RuntimeCommand::ForkSession { harness, .. } => Some(*harness),
        _ => None,
    }
}

fn cancel_pending_command(pending: &mut Option<RuntimeCommand>) -> bool {
    pending.take().is_some()
}

fn take_trust_command(
    pending: &mut Option<RuntimeCommand>,
    trusted: bool,
) -> Option<RuntimeCommand> {
    pending
        .take()
        .filter(|_| trusted)
        .map(restart_session_after_trust)
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
#[path = "trust_tests.rs"]
mod tests;
