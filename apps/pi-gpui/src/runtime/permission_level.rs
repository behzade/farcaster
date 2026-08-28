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

    pub(crate) fn flag_value(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Sandboxed => "sandboxed",
            Self::Full => "full",
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

    pub(crate) fn flag_value(self) -> &'static str {
        match self {
            Self::Sandboxed => "sandboxed",
            Self::Full => "full",
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

struct PendingPermissionChange {
    request_id: String,
    level: PermissionLevel,
}

#[derive(Default)]
pub(super) struct PermissionChangeState {
    pending: Option<PendingPermissionChange>,
    generation: u64,
    command_id: Option<String>,
}

impl PermissionChangeState {
    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.command_id = None;
    }

    pub(super) fn is_idle(&self) -> bool {
        self.pending.is_none() && self.command_id.is_none()
    }
}

impl RuntimeOwner {
    pub(super) fn set_permission_level(&mut self, level: PermissionLevel) {
        if !self.permission_changes.is_idle() || self.process_command.permission_level == level {
            return;
        }
        if self.active_snapshot().conversation.running
            || self.active_snapshot().conversation.compacting
        {
            self.fail_permission_change(
                "Wait for the current response to finish before changing sandbox access".into(),
            );
            return;
        }

        self.permission_changes.generation = self.permission_changes.generation.saturating_add(1);
        let request_id = format!("gpui-permission-{}", self.permission_changes.generation);
        let request = json!({
            "requestId": request_id,
            "files": level.files.flag_value(),
            "network": level.network.flag_value(),
        });
        let command = json!({
            "type": "prompt",
            "message": format!("/sandbox-mode {request}"),
        });
        let rpc_id = match self
            .process
            .as_mut()
            .map(|process| process.send_command(command))
        {
            Some(Ok(id)) => id,
            Some(Err(error)) => {
                self.fail(error);
                return;
            }
            None => {
                self.fail("Cannot change sandbox access: Pi is not connected".into());
                return;
            }
        };
        self.permission_changes.command_id = Some(rpc_id);
        self.permission_changes.pending = Some(PendingPermissionChange { request_id, level });
        self.active_snapshot_mut().status = "Changing sandbox access".into();
        self.publish();
    }

    pub(super) fn apply_sandbox_mode_result(
        &mut self,
        result: Result<crate::protocol::SandboxModeResult, String>,
    ) {
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if self.permission_changes.pending.is_some() {
                    self.fail_permission_change(error);
                }
                return;
            }
        };
        let Some(pending) = self.permission_changes.pending.as_ref() else {
            return;
        };
        if result.request_id != pending.request_id {
            return;
        }
        if result.version != 1
            || result.files != pending.level.files.flag_value()
            || result.network != pending.level.network.flag_value()
        {
            self.fail_permission_change(
                "Sandbox mode acknowledgement did not match the request".into(),
            );
            return;
        }
        if !result.success {
            self.fail_permission_change(
                result
                    .error
                    .unwrap_or_else(|| "pi-nono rejected the sandbox mode change".into()),
            );
            return;
        }

        let level = pending.level;
        self.permission_changes.pending = None;
        self.process_command.permission_level = level;
        self.active_snapshot_mut().status = "Ready".into();
        self.publish();
    }

    pub(super) fn apply_permission_command_response(
        &mut self,
        response: &crate::protocol::RpcResponse,
    ) -> bool {
        let Some(id) = response.id.as_ref() else {
            return false;
        };
        if self.permission_changes.command_id.as_deref() != Some(id) {
            return false;
        }
        self.permission_changes.command_id = None;
        if !response.success && self.permission_changes.pending.is_some() {
            self.fail_permission_change(
                response
                    .error
                    .clone()
                    .unwrap_or_else(|| "pi-nono did not accept the sandbox mode command".into()),
            );
        }
        true
    }

    fn fail_permission_change(&mut self, message: String) {
        self.permission_changes.pending = None;
        let snapshot = self.active_snapshot_mut();
        snapshot.status = "Permissions unchanged".into();
        conversation_mut(snapshot).push_local_error("Permissions unchanged", message);
        self.publish();
    }
}
