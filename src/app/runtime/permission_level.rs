//! Runtime policy for applying agent filesystem and network access changes.

use super::*;

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
        if let Err(error) = crate::agents::validate_launch(&next_command, &self.project) {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Permissions unchanged".into();
            conversation_mut(snapshot).push_local_error("Permissions unchanged", error);
            self.publish();
            return;
        }
        self.process_command = next_command;
        self.restart_process_preserving_transcript();
    }

    pub(super) fn set_app_proxy(&mut self, proxy: Option<String>) {
        if self.process_command.app_proxy == proxy {
            return;
        }
        let previous = std::mem::replace(&mut self.process_command.app_proxy, proxy);
        if !self.permission_change_ready() {
            self.permission_changes.queue_reload();
            self.publish();
            return;
        }
        if self.process.is_none() {
            self.publish();
            return;
        }
        if let Err(error) = crate::agents::validate_launch(&self.process_command, &self.project) {
            self.process_command.app_proxy = previous;
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Proxy unchanged".into();
            conversation_mut(snapshot).push_local_error("Proxy unchanged", error);
            self.publish();
            return;
        }
        self.restart_process_preserving_transcript();
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
        if let Err(error) = crate::agents::validate_launch(&self.process_command, &self.project) {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Sandbox grants inactive".into();
            conversation_mut(snapshot).push_local_error("Sandbox grants inactive", error);
            self.publish();
            return;
        }
        self.restart_process_preserving_transcript();
    }

    pub(super) fn activate_sandbox_grant(&mut self) {
        if self.sandbox_grant_handoff.is_some() {
            return;
        }
        self.sandbox_grant_handoff = Some(super::SandboxGrantHandoff::WaitingForSiblingTools);
        self.active_snapshot_mut().status = "Activating sandbox access".into();
        self.maybe_interrupt_for_sandbox_grant();
        self.publish();
    }

    pub(super) fn maybe_interrupt_for_sandbox_grant(&mut self) {
        if self.sandbox_grant_handoff != Some(super::SandboxGrantHandoff::WaitingForSiblingTools) {
            return;
        }
        let only_access_request_remains = self
            .active_tool_calls
            .values()
            .all(|name| name == "request_access" || name.ends_with("_request_access"));
        if !only_access_request_remains {
            return;
        }
        self.sandbox_grant_handoff = Some(super::SandboxGrantHandoff::Interrupting);
        self.send(crate::agents::SessionCommand::Abort);
    }

    pub(super) fn finish_sandbox_grant_handoff(&mut self) {
        if self.sandbox_grant_handoff.take().is_none() {
            return;
        }
        let error = if self.active_session.is_none() {
            Some("The interrupted agent session could not be resumed".into())
        } else {
            crate::agents::validate_launch(&self.process_command, &self.project).err()
        };
        if let Some(error) = error {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Sandbox grant inactive".into();
            conversation_mut(snapshot).push_local_error("Sandbox grant inactive", error);
            self.publish();
            return;
        }
        self.deferred_prompt = Some(super::DeferredPrompt {
            mode: crate::protocol::PromptMode::Normal,
            message: crate::agents::sandbox_grant_continuation(),
            images: Vec::new(),
            outbox_id: None,
        });
        self.restart_process_preserving_transcript();
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
        _response: &crate::agents::SessionResponse,
    ) -> bool {
        false
    }
}
