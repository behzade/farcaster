//! Runtime policy for applying harness-native access mode changes.

use super::*;

const ACCESS_MODE_CHANGE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct AccessModeChangeState {
    queued: Option<HarnessAccessMode>,
    apply_due: Option<Instant>,
    restart_pending: bool,
}

impl AccessModeChangeState {
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.queued.is_none() && !self.restart_pending
    }

    fn queue(&mut self, requested: HarnessAccessMode, effective: HarnessAccessMode) {
        self.queued = (requested != effective).then_some(requested);
        self.apply_due = self
            .queued
            .is_some()
            .then(|| Instant::now() + ACCESS_MODE_CHANGE_DEBOUNCE);
    }

    fn take_queued_if_due(&mut self, now: Instant) -> Option<HarnessAccessMode> {
        if self.apply_due.is_none_or(|due| now < due) {
            return None;
        }
        self.apply_due = None;
        self.queued.take()
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.apply_due
    }

    #[cfg(test)]
    pub(super) fn make_due(&mut self) {
        self.apply_due = self.queued.is_some().then(Instant::now);
    }

    fn queue_restart(&mut self) {
        self.restart_pending = true;
    }

    fn take_restart(&mut self) -> bool {
        std::mem::take(&mut self.restart_pending)
    }

    pub(super) fn requested_mode(&self, effective: HarnessAccessMode) -> HarnessAccessMode {
        self.queued.unwrap_or(effective)
    }

    pub(super) fn take_requested_mode(
        &mut self,
        effective: HarnessAccessMode,
    ) -> HarnessAccessMode {
        self.apply_due = None;
        self.queued.take().unwrap_or(effective)
    }
}

impl RuntimeOwner {
    pub(super) fn access_mode_change_ready(&self) -> bool {
        let conversation = &self.active_snapshot().conversation;
        !conversation.running && !conversation.compacting
    }

    pub(super) fn set_access_mode(&mut self, mode: HarnessAccessMode) {
        if crate::agents::normalize_access_mode(&self.harness, mode) != mode {
            return;
        }
        if self.process.is_none() && self.access_mode_change_ready() {
            self.access_mode_changes.queued = None;
            self.access_mode_changes.apply_due = None;
            self.process_command.access_mode = mode;
            self.publish();
            return;
        }
        self.access_mode_changes
            .queue(mode, self.process_command.access_mode);
        self.publish();
    }

    fn apply_access_mode(&mut self, mode: HarnessAccessMode) {
        if self.process_command.access_mode == mode {
            return;
        }
        let mut next_command = self.process_command.clone();
        next_command.access_mode = mode;
        if let Err(error) = crate::agents::validate_launch(&next_command, &self.project) {
            let snapshot = self.active_snapshot_mut();
            snapshot.status = "Access mode unchanged".into();
            conversation_mut(snapshot).push_local_error("Access mode unchanged", error);
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
        if !self.access_mode_change_ready() {
            self.access_mode_changes.queue_restart();
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

    pub(super) fn apply_queued_access_mode_change(&mut self) {
        if !self.access_mode_change_ready() {
            return;
        }
        if let Some(mode) = self.access_mode_changes.take_queued_if_due(Instant::now()) {
            let _ = self.access_mode_changes.take_restart();
            self.apply_access_mode(mode);
        } else if self.access_mode_changes.queued.is_none()
            && self.access_mode_changes.take_restart()
            && self.process.is_some()
        {
            self.restart_process_preserving_transcript();
        }
    }
}
