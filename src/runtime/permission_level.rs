//! Independent filesystem and network access modes for the Pi child process.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FileAccessMode {
    ReadOnly,
    #[default]
    Sandboxed,
    Full,
}

impl FileAccessMode {
    pub(crate) fn all() -> [Self; 3] {
        [Self::ReadOnly, Self::Sandboxed, Self::Full]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::Sandboxed => "Sandboxed",
            Self::Full => "Full",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum NetworkAccessMode {
    #[default]
    Sandboxed,
    Full,
}

impl NetworkAccessMode {
    pub(crate) fn all() -> [Self; 2] {
        [Self::Sandboxed, Self::Full]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sandboxed => "Sandboxed",
            Self::Full => "Full",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PermissionLevel {
    pub(crate) files: FileAccessMode,
    pub(crate) network: NetworkAccessMode,
}

impl PermissionLevel {
    pub(crate) fn label(self) -> String {
        format!(
            "Files: {} · Network: {}",
            self.files.label(),
            self.network.label()
        )
    }

    pub(crate) fn with_files(self, files: FileAccessMode) -> Self {
        Self { files, ..self }
    }

    pub(crate) fn with_network(self, network: NetworkAccessMode) -> Self {
        Self { network, ..self }
    }
}

#[derive(Default)]
pub(super) struct PermissionChangeState {
    queued: Option<PermissionLevel>,
    reload_sandbox: bool,
}

impl PermissionChangeState {
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.queued.is_none() && !self.reload_sandbox
    }

    fn queue(&mut self, requested: PermissionLevel, effective: PermissionLevel) {
        self.queued = (requested != effective).then_some(requested);
    }

    fn take_queued(&mut self) -> Option<PermissionLevel> {
        self.queued.take()
    }

    fn queue_reload(&mut self) {
        self.reload_sandbox = true;
    }

    fn take_reload(&mut self) -> bool {
        std::mem::take(&mut self.reload_sandbox)
    }

    pub(super) fn requested_level(&self, effective: PermissionLevel) -> PermissionLevel {
        self.queued.unwrap_or(effective)
    }

    pub(super) fn take_requested_level(&mut self, effective: PermissionLevel) -> PermissionLevel {
        self.queued.take().unwrap_or(effective)
    }
}

impl RuntimeOwner {
    fn permission_change_ready(&self) -> bool {
        let conversation = &self.active_snapshot().conversation;
        !conversation.running && !conversation.compacting
    }

    pub(super) fn set_permission_level(&mut self, level: PermissionLevel) {
        if !self.permission_change_ready() {
            self.permission_changes
                .queue(level, self.process_command.permission_level);
            self.publish();
            return;
        }
        if self.process_command.permission_level == level {
            return;
        }
        let mut next_command = self.process_command.clone();
        next_command.permission_level = level;
        if self.process.is_none() {
            self.process_command = next_command;
            self.publish();
            return;
        }
        if let Err(error) = next_command.command(&self.project) {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Permissions unchanged".into();
            conversation_mut(snapshot).push_local_error("Permissions unchanged", error);
            self.publish();
            return;
        }
        self.process_command = next_command;
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process(session);
    }

    pub(super) fn reload_sandbox_grants(&mut self) {
        if !self.permission_change_ready() {
            self.permission_changes.queue_reload();
            self.publish();
            return;
        }
        if self.process.is_none() {
            return;
        }
        if let Err(error) = self.process_command.command(&self.project) {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Sandbox grants inactive".into();
            conversation_mut(snapshot).push_local_error("Sandbox grants inactive", error);
            self.publish();
            return;
        }
        let session = if self.snapshot.history_preview {
            self.snapshot.selected_session.clone()
        } else {
            self.active_session.clone()
        };
        self.start_process(session);
    }

    pub(super) fn apply_queued_permission_change(&mut self) {
        if !self.permission_change_ready() {
            return;
        }
        if let Some(level) = self.permission_changes.take_queued() {
            let _ = self.permission_changes.take_reload();
            self.set_permission_level(level);
        } else if self.permission_changes.take_reload() {
            self.reload_sandbox_grants();
        }
    }

    pub(super) fn apply_sandbox_mode_result(
        &mut self,
        _result: Result<crate::protocol::SandboxModeResult, String>,
    ) {
    }

    pub(super) fn apply_permission_command_response(
        &mut self,
        _response: &crate::protocol::RpcResponse,
    ) -> bool {
        false
    }
}
