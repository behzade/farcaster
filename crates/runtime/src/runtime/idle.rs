use super::*;

pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CLEANUP_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct IdleRetirement {
    pub(super) deadline: Option<Instant>,
    wake: Option<(Instant, Box<dyn crate::ScheduledWake>)>,
    activity: Option<(PathBuf, Box<dyn crate::WorkerActivity>)>,
    blocked: bool,
    pub(super) inbox: Option<Box<dyn agents::SessionInbox>>,
    pub(super) resume_attempted: bool,
    worker_revision: Option<u64>,
    pub(super) requests: HashSet<String>,
    dialogs: HashSet<String>,
    children: HashSet<String>,
    pub(super) closing: Option<Box<dyn SessionTransport>>,
    pub(super) retired: bool,
    resume_commands: VecDeque<SessionCommand>,
}

impl IdleRetirement {
    pub(super) fn invalidate(&mut self) {
        self.deadline = None;
        self.blocked = false;
    }

    fn schedule(&mut self, host: &dyn RuntimeHost, now: Instant) {
        if self.wake.is_none()
            && let Some(deadline) = self.deadline
        {
            self.wake = Some((
                deadline,
                host.schedule_wake(deadline.max(now), thread::current()),
            ));
        }
    }

    pub(super) fn observe(&mut self, event: &SessionEvent) {
        if !matches!(event, SessionEvent::Stderr(_)) {
            self.invalidate();
        }
        match event {
            SessionEvent::Response(response) => {
                if let Some(id) = &response.id {
                    self.requests.remove(id);
                }
            }
            SessionEvent::Interaction(request) => {
                if let Some(id) = request.dialog_id() {
                    self.dialogs.insert(id.to_owned());
                }
            }
            SessionEvent::Activity(event)
                if event.kind() == &SessionActivityKind::ChildSessionsChanged =>
            {
                if let Some(id) = event.value().pointer("/child/id").and_then(Value::as_str) {
                    if event
                        .value()
                        .pointer("/child/is_running")
                        .and_then(Value::as_bool)
                        == Some(false)
                    {
                        self.children.remove(id);
                    } else {
                        self.children.insert(id.to_owned());
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn responded(&mut self, response: &ExtensionUiResponse) {
        let (ExtensionUiResponse::Value { id, .. }
        | ExtensionUiResponse::Confirmed { id, .. }
        | ExtensionUiResponse::Cancelled { id, .. }) = response;
        self.dialogs.remove(id);
    }
}

impl RuntimeOwner {
    pub(super) fn send_idle_aware(&mut self, command: SessionCommand) {
        if self.idle_retirement.retired {
            self.restart_process_preserving_transcript();
            if self.process.is_none() {
                return;
            }
            self.idle_retirement.resume_commands.push_back(command);
        } else if !self.idle_retirement.resume_commands.is_empty() {
            self.idle_retirement.resume_commands.push_back(command);
        } else {
            self.send(command);
        }
    }

    pub(super) fn send_resumed_commands(&mut self) {
        if !self.startup_state_loaded
            || !self.startup_history_loaded
            || self.pending_session_controls.selection_pending()
        {
            return;
        }
        while self.process.is_some() {
            let Some(command) = self.idle_retirement.resume_commands.pop_front() else {
                break;
            };
            self.send(command);
        }
    }

    fn can_retire_idle_process(&self) -> bool {
        let snapshot = self.active_snapshot();
        self.process
            .as_ref()
            .is_some_and(|process| process.can_retire())
            && self.active_session.is_some()
            && agents::supports_session_resume(self.harness)
            && self.startup_state_loaded
            && self.startup_history_loaded
            && self.access_mode_change_ready()
            && self.access_mode_changes.is_idle()
            && !self.access_mode_changes.applying
            && !self.pending_session_controls.selection_pending()
            && self.pending_prompt.is_none()
            && self.pending_queued_prompts.is_empty()
            && self.retired_prompts.is_empty()
            && self.queued_prompts.is_empty()
            && self.saved_prompts.is_empty()
            && !self.title_generation.in_flight
            && self.idle_retirement.requests.is_empty()
            && self.idle_retirement.dialogs.is_empty()
            && self.idle_retirement.children.is_empty()
            && self.idle_retirement.resume_commands.is_empty()
            && self.idle_retirement.inbox.is_none()
            && self.idle_retirement.closing.is_none()
            && snapshot.session.as_ref().is_some_and(|session| {
                !session.is_streaming
                    && !session.is_compacting
                    && session.pending_message_count == 0
            })
            && snapshot
                .session_goal
                .as_ref()
                .is_none_or(|goal| goal.status != "active")
            && snapshot.conversation.queue.steering.is_empty()
            && snapshot.conversation.queue.follow_up.is_empty()
    }

    pub(super) fn poll_idle_retirement(&mut self, now: Instant, system_woke: bool) {
        if self
            .idle_retirement
            .wake
            .as_ref()
            .is_some_and(|(due, _)| now >= *due)
        {
            self.idle_retirement.wake = None;
        }
        if self.idle_retirement.closing.is_some() {
            self.poll_idle_cleanup(now);
            return;
        }
        if self.process.is_some()
            && self.startup_state_loaded
            && self.startup_history_loaded
            && self
                .idle_retirement
                .inbox
                .as_ref()
                .is_some_and(|inbox| inbox.transferred())
        {
            self.idle_retirement.inbox = None;
        }
        if self.process.is_none()
            && !self.idle_retirement.resume_attempted
            && self
                .idle_retirement
                .inbox
                .as_ref()
                .is_some_and(|inbox| inbox.has_pending_messages())
        {
            self.idle_retirement.resume_attempted = true;
            self.start_process_from(self.active_session.clone(), None, true);
        }
        if !self.can_retire_idle_process() {
            self.idle_retirement.invalidate();
            self.idle_retirement.activity = None;
            return;
        }
        let project = &self.active_snapshot().project;
        if self
            .idle_retirement
            .activity
            .as_ref()
            .is_none_or(|(subscribed, _)| subscribed != project)
        {
            self.idle_retirement.activity = Some((
                project.clone(),
                self.host
                    .subscribe_worker_activity(project, thread::current()),
            ));
            self.idle_retirement.worker_revision = None;
        }
        let revision = self
            .idle_retirement
            .activity
            .as_ref()
            .expect("subscribed")
            .1
            .revision();
        if self.idle_retirement.worker_revision.replace(revision) != Some(revision) {
            self.idle_retirement.invalidate();
        }
        if !system_woke && self.idle_retirement.blocked {
            return;
        }
        let deadline = *self
            .idle_retirement
            .deadline
            .get_or_insert(now + IDLE_TIMEOUT);
        if !system_woke && now < deadline {
            self.idle_retirement.schedule(&*self.host, now);
            return;
        }
        let _check = self.host.timer(RuntimeMetric::IdleRetirement);
        let workers_idle = self.host.worker_snapshots().is_ok_and(|workers| {
            workers.iter().all(|worker| {
                worker.project != self.active_snapshot().project
                    || (matches!(
                        worker.status,
                        agents::WorkerStatus::Idle
                            | agents::WorkerStatus::Stopped
                            | agents::WorkerStatus::Failed
                    ) && worker.pending_input.is_none())
            })
        });
        let conversation = &self.active_snapshot().conversation;
        let tools_idle = conversation.items.iter().all(|item| {
            item.kind != crate::conversation::TranscriptKind::Tool
                || !matches!(
                    item.tool_execution_state(),
                    Some(
                        crate::conversation::ToolExecutionState::Pending
                            | crate::conversation::ToolExecutionState::Running
                    )
                )
        });
        let inputs_idle = self
            .host
            .worker_inboxes_idle(&self.active_snapshot().project);
        let revision_after = self
            .idle_retirement
            .activity
            .as_ref()
            .expect("subscribed")
            .1
            .revision();
        if revision_after != revision {
            self.idle_retirement.worker_revision = Some(revision_after);
            self.idle_retirement.invalidate();
            self.idle_retirement.deadline = Some(now + IDLE_TIMEOUT);
            self.idle_retirement.schedule(&*self.host, now);
            return;
        }
        if !workers_idle || !tools_idle || conversation.has_pending_receipts() || !inputs_idle {
            self.idle_retirement.deadline = None;
            self.idle_retirement.blocked = true;
            return;
        }
        let inbox = match self
            .process
            .as_ref()
            .expect("checked live process")
            .retain_inbox()
        {
            Ok(inbox) => inbox,
            Err(error) => {
                self.idle_retirement.deadline = None;
                zlog::warn!("Could not retain idle session inbox: {error}");
                return;
            }
        };
        if inbox
            .as_ref()
            .is_some_and(|inbox| inbox.has_pending_messages())
        {
            self.idle_retirement.deadline = None;
            return;
        }
        self.idle_retirement.inbox = inbox;
        self.idle_retirement.closing = self.process.take();
        self.idle_retirement.deadline = None;
        self.idle_retirement.wake = None;
        self.idle_retirement.activity = None;
        self.idle_retirement.retired = true;
        self.poll_idle_cleanup(now);
    }

    fn poll_idle_cleanup(&mut self, now: Instant) {
        let process = self
            .idle_retirement
            .closing
            .as_mut()
            .expect("closing process");
        let retry = self.idle_retirement.deadline.is_none_or(|due| now >= due);
        let exited = process.has_exited();
        let closed = if !exited && retry {
            let result = process.close();
            if let Err(error) = &result {
                zlog::warn!("Idle harness cleanup failed: {error}");
            }
            result.is_ok() || process.has_exited()
        } else {
            exited
        };
        if closed {
            self.finish_idle_retirement();
            return;
        }
        if retry {
            self.idle_retirement.deadline = Some(now + CLEANUP_RETRY_INTERVAL);
        }
        self.idle_retirement.schedule(&*self.host, now);
        if self.active_snapshot().connected {
            self.active_snapshot_mut().connected = false;
            self.publish();
        }
    }

    fn finish_idle_retirement(&mut self) {
        self.idle_retirement.closing = None;
        self.idle_retirement.deadline = None;
        self.idle_retirement.wake = None;
        self.idle_retirement.activity = None;
        self.idle_retirement.retired = true;
        self.idle_retirement.resume_attempted = false;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.active_snapshot_mut().connected = false;
        zlog::info!(
            "Retired idle {} session {:?}",
            self.backend_name(),
            self.active_session
        );
        self.publish();
        if self
            .idle_retirement
            .inbox
            .as_ref()
            .is_some_and(|inbox| inbox.has_pending_messages())
        {
            thread::current().unpark();
        }
    }
}

#[cfg(test)]
#[path = "idle_tests.rs"]
mod tests;
