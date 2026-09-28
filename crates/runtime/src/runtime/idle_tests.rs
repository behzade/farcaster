use super::*;
use std::{cell::Cell, rc::Rc};

struct Transport {
    closes: Rc<Cell<usize>>,
    fail_close: bool,
}

impl SessionTransport for Transport {
    fn can_retire(&self) -> bool {
        true
    }
    fn send(&mut self, _: SessionCommand) -> Result<String, String> {
        Ok("request".into())
    }
    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<SessionEvent> {
        None
    }
    fn close(&mut self) -> Result<(), String> {
        self.closes.set(self.closes.get() + 1);
        if self.fail_close {
            Err("still alive".into())
        } else {
            Ok(())
        }
    }
}

fn ready_owner() -> (RuntimeOwner, Rc<Cell<usize>>) {
    static NEXT_PROJECT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT_PROJECT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut owner, _) =
        super::super::tests::owner_without_process(PathBuf::from(format!("/idle-test/{id}")));
    let closes = Rc::new(Cell::new(0));
    owner.process = Some(Box::new(Transport {
        closes: closes.clone(),
        fail_close: false,
    }));
    owner.active_session = Some(owner.project.join("session.jsonl"));
    owner.snapshot.selected_session = owner.active_session.clone();
    owner.snapshot.session = Some(
        serde_json::from_value(json!({
            "isStreaming":false, "isCompacting":false, "sessionId":"idle-session",
            "autoCompactionEnabled":true, "messageCount":1, "pendingMessageCount":0
        }))
        .expect("session state"),
    );
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    (owner, closes)
}

#[test]
fn busy_or_unresumable_sessions_survive_timeout_and_system_wake() {
    let blockers: &[fn(&mut RuntimeOwner)] = &[
        |o| conversation_mut(&mut o.snapshot).running = true,
        |o| conversation_mut(&mut o.snapshot).compacting = true,
        |o| conversation_mut(&mut o.snapshot).retrying = true,
        |o| o.normal_prompt_in_flight = true,
        |o| o.pending_prompt_id = Some("unconfirmed".into()),
        |o| o.pending_outbox_id = Some(1),
        |o| o.active_session = None,
        |o| o.startup_history_loaded = false,
        |o| o.snapshot.session.as_mut().unwrap().pending_message_count = 1,
        |o| {
            o.snapshot.session_goal = Some(agents::SessionGoal {
                objective: "keep working".into(),
                status: "active".into(),
                token_budget: None,
                tokens_used: 0,
                time_used_seconds: 0,
            })
        },
        |o| {
            conversation_mut(&mut o.snapshot)
                .queue
                .follow_up
                .push("queued".into())
        },
    ];
    for (index, block) in blockers.iter().enumerate() {
        let (mut owner, closes) = ready_owner();
        let now = Instant::now();
        owner.poll_idle_retirement(now, false);
        block(&mut owner);
        owner.poll_idle_retirement(now + IDLE_TIMEOUT, false);
        owner.apply_command(RuntimeCommand::SystemWake);
        assert_eq!(closes.get(), 0, "blocker {index}");
        assert!(owner.process.is_some());
    }
}

#[test]
fn approval_and_native_child_block_retirement_even_between_turns() {
    let (mut owner, closes) = ready_owner();
    owner.apply_process_item(SessionEvent::Interaction(ExtensionUiRequest::Confirm {
        id: "approval".into(),
        title: "Allow?".into(),
        message: "Run command".into(),
        timeout: None,
    }));
    owner.apply_command(RuntimeCommand::SystemWake);
    assert_eq!(closes.get(), 0);
    owner.apply_command(RuntimeCommand::ExtensionResponse(
        ExtensionUiResponse::Confirmed {
            id: "approval".into(),
            confirmed: true,
        },
    ));
    owner.idle_retirement.observe(&SessionEvent::Activity(
        json!({
            "type":"child_sessions_changed", "child":{"id":"child", "is_running":true}
        })
        .into(),
    ));
    owner.apply_command(RuntimeCommand::SystemWake);
    assert_eq!(closes.get(), 0);
    owner.idle_retirement.observe(&SessionEvent::Activity(
        json!({
            "type":"child_sessions_changed", "child":{"id":"child", "is_running":false}
        })
        .into(),
    ));
    owner.apply_command(RuntimeCommand::SystemWake);
    assert_eq!(closes.get(), 1);
}

#[test]
fn pending_rpc_blocks_retirement_and_reply_starts_a_fresh_idle_period() {
    let (mut owner, closes) = ready_owner();
    let now = Instant::now();
    owner.poll_idle_retirement(now, false);
    owner.send(SessionCommand::Rename {
        name: "new name".into(),
    });
    owner.poll_idle_retirement(now + IDLE_TIMEOUT, false);
    assert_eq!(closes.get(), 0);
    owner
        .idle_retirement
        .observe(&SessionEvent::Response(agents::SessionResponse::success(
            Some("request".into()),
            agents::SessionResponsePayload::Rename,
        )));
    owner.poll_idle_retirement(now + IDLE_TIMEOUT, false);
    assert_eq!(closes.get(), 0);
    owner.poll_idle_retirement(now + IDLE_TIMEOUT * 2, false);
    assert_eq!(closes.get(), 1);
}

#[test]
fn failed_cleanup_keeps_process_out_of_polling_and_retries_only_at_deadline() {
    let (mut owner, closes) = ready_owner();
    owner.process = Some(Box::new(Transport {
        closes: closes.clone(),
        fail_close: true,
    }));
    let now = Instant::now();
    owner.poll_idle_retirement(now, true);
    owner.poll_idle_retirement(now, true);
    owner.poll_idle_retirement(
        now + CLEANUP_RETRY_INTERVAL - Duration::from_millis(1),
        false,
    );
    assert_eq!(closes.get(), 1);
    assert!(owner.process.is_none());
    assert!(owner.idle_retirement.closing.is_some());
    assert!(!owner.snapshot.connected);
    assert!(owner.idle_retirement.wake.is_some());
    owner.poll_idle_retirement(now + CLEANUP_RETRY_INTERVAL, false);
    assert_eq!(closes.get(), 2);
    assert_eq!(
        owner.idle_retirement.deadline,
        Some(now + CLEANUP_RETRY_INTERVAL * 2)
    );
    assert!(owner.idle_retirement.wake.is_some());
}

#[test]
fn worker_activity_resets_the_full_idle_period_even_between_polls() {
    let (mut owner, closes) = ready_owner();
    let workers = Arc::new(std::sync::Mutex::new((
        0,
        vec![agents::WorkerSnapshot {
            id: "unrelated-worker".into(),
            backend: Backend::Pi,
            project: owner.project.clone(),
            session_key: None,
            session_locator: None,
            status: agents::WorkerStatus::Running,
            output: None,
            error: None,
            pending_input: None,
        }],
        false,
    )));
    let (host, metrics) = crate::test_support::host_with_idle_metrics(workers.clone());
    owner.host = host;
    let now = Instant::now();
    owner.poll_idle_retirement(now, false);
    assert_eq!(metrics.checks.load(std::sync::atomic::Ordering::Relaxed), 0);
    owner.poll_idle_retirement(now + IDLE_TIMEOUT, false);
    assert!(
        owner.idle_retirement.deadline.is_none(),
        "busy workers block expiry"
    );
    assert!(owner.idle_retirement.wake.is_none());
    owner.poll_idle_retirement(now + IDLE_TIMEOUT * 2, false);
    assert_eq!(
        metrics.checks.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "blocked sessions wait for activity"
    );
    {
        let mut workers = workers.lock().unwrap();
        workers.0 += 1;
        workers.1[0].status = agents::WorkerStatus::Idle;
    }
    let settled = now + IDLE_TIMEOUT * 2 + Duration::from_secs(1);
    owner.poll_idle_retirement(settled, false);
    assert_eq!(owner.idle_retirement.deadline, Some(settled + IDLE_TIMEOUT));
    {
        let mut workers = workers.lock().unwrap();
        workers.0 += 3;
        workers.1.clear();
    }
    let observed = settled + IDLE_TIMEOUT;
    owner.poll_idle_retirement(observed, false);
    assert_eq!(closes.get(), 0);
    assert_eq!(
        owner.idle_retirement.deadline,
        Some(observed + IDLE_TIMEOUT)
    );
    workers.lock().unwrap().2 = true;
    owner.poll_idle_retirement(observed + IDLE_TIMEOUT, false);
    assert_eq!(
        closes.get(),
        0,
        "activity within snapshot collection invalidates expiry"
    );
    owner.poll_idle_retirement(observed + IDLE_TIMEOUT * 2, false);
    assert_eq!(closes.get(), 1);
}

#[test]
fn reports_during_and_after_close_survive_runtime_retirement() {
    check_reports_survive_close(CloseOutcome::Complete);
}

#[test]
fn reports_survive_cleanup_error_after_confirmed_exit() {
    check_reports_survive_close(CloseOutcome::ErrorAfterExit);
}

#[test]
fn reports_survive_cleanup_error_until_later_exit() {
    check_reports_survive_close(CloseOutcome::ErrorThenExit);
}

#[test]
fn reports_survive_cleanup_error_and_scheduled_retry() {
    check_reports_survive_close(CloseOutcome::ErrorThenRetry);
}

#[derive(Clone, Copy)]
enum CloseOutcome {
    Complete,
    ErrorAfterExit,
    ErrorThenExit,
    ErrorThenRetry,
}

fn check_reports_survive_close(outcome: CloseOutcome) {
    struct ReportingTransport {
        registry: agents::CallerRegistry,
        parent: agents::CallerIdentity,
        child: agents::CallerIdentity,
        outcome: CloseOutcome,
        exited: Rc<Cell<bool>>,
        reported: bool,
        close_attempts: usize,
    }
    impl SessionTransport for ReportingTransport {
        fn has_exited(&mut self) -> bool {
            self.exited.get()
        }
        fn can_retire(&self) -> bool {
            !self.parent.has_pending_messages()
        }
        fn retain_inbox(&self) -> Result<Option<Box<dyn agents::SessionInbox>>, String> {
            self.parent.retain_inbox()
        }
        fn send(&mut self, _: SessionCommand) -> Result<String, String> {
            unreachable!()
        }
        fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
            unreachable!()
        }
        fn poll(&mut self) -> Option<SessionEvent> {
            self.parent
                .try_recv()
                .map(|_| SessionEvent::Failure("report consumed by closed writer".into()))
        }
        fn close(&mut self) -> Result<(), String> {
            self.close_attempts += 1;
            if !self.reported {
                self.registry
                    .send(self.child.token(), "parent", "during close".into())?;
                self.reported = true;
            }
            match self.outcome {
                CloseOutcome::Complete => Ok(()),
                CloseOutcome::ErrorAfterExit => {
                    self.exited.set(true);
                    Err("session close failed; process reaped".into())
                }
                CloseOutcome::ErrorThenRetry if self.close_attempts > 1 => Ok(()),
                _ => Err("exit unconfirmed".into()),
            }
        }
    }
    let (mut owner, _) = ready_owner();
    let registry = agents::CallerRegistry::default();
    let profile = agents::CallerProfile {
        backend: Backend::Pi,
        provider: None,
        model: None,
        effort: None,
    };
    let parent = registry.issue(&owner.project, profile.clone(), None);
    let path = owner
        .active_session
        .as_ref()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    parent.bind(path.clone());
    let parent_id = parent.worker_identity().unwrap().0;
    let make_child = |name: &str| {
        registry
            .issue_as_with_access(
                &owner.project,
                profile.clone(),
                None,
                name.into(),
                name.into(),
                Some(parent_id.clone()),
                agents::HarnessAccessMode::Auto,
            )
            .unwrap()
    };
    let after_close = make_child("after-close");
    let child = make_child("during-close");
    let exited = Rc::new(Cell::new(false));
    owner.process = Some(Box::new(ReportingTransport {
        registry: registry.clone(),
        parent,
        child,
        outcome,
        exited: exited.clone(),
        reported: false,
        close_attempts: 0,
    }));
    let now = Instant::now();
    owner.poll_idle_retirement(now, true);
    if matches!(
        outcome,
        CloseOutcome::ErrorThenExit | CloseOutcome::ErrorThenRetry
    ) {
        assert!(
            owner
                .process
                .as_mut()
                .and_then(|process| process.poll())
                .is_none()
        );
        assert!(owner.idle_retirement.closing.is_some());
        assert!(
            owner
                .idle_retirement
                .inbox
                .as_ref()
                .unwrap()
                .has_pending_messages()
        );
        owner.poll_idle_retirement(now, false);
        owner.start_process_from(owner.active_session.clone(), None, true);
        assert!(owner.process.is_none());
        assert!(owner.idle_retirement.closing.is_some());
        if matches!(outcome, CloseOutcome::ErrorThenRetry) {
            owner.poll_idle_retirement(now + CLEANUP_RETRY_INTERVAL, false);
        } else {
            exited.set(true);
            owner.poll_idle_retirement(now, false);
        }
    }
    assert!(owner.process.is_none());
    assert!(owner.idle_retirement.closing.is_none());
    registry
        .send(after_close.token(), "parent", "after close".into())
        .unwrap();
    assert!(
        owner
            .idle_retirement
            .inbox
            .as_ref()
            .unwrap()
            .has_pending_messages()
    );
    let resumed = registry.issue(&owner.project, profile, None);
    resumed.bind(path);
    assert!(owner.idle_retirement.inbox.as_ref().unwrap().transferred());
    assert_eq!(resumed.try_recv().unwrap().message, "during close");
    assert_eq!(resumed.try_recv().unwrap().message, "after close");
    assert!(resumed.try_recv().is_none());
}

#[test]
fn many_quiet_sessions_do_no_scans_or_timer_churn_before_expiry() {
    use std::sync::atomic::Ordering;
    let workers = Arc::new(std::sync::Mutex::new((0, Vec::new(), false)));
    let (host, metrics) = crate::test_support::host_with_idle_metrics(workers.clone());
    let (mut template, _) = ready_owner();
    for index in 0..2_000 {
        conversation_mut(&mut template.snapshot).push_local_user(
            format!("history {index}"),
            0,
            false,
        );
    }
    let mut sessions: Vec<_> = (0..64)
        .map(|_| {
            let (mut owner, closes) = ready_owner();
            owner.host = host.clone();
            owner.snapshot.conversation = template.snapshot.conversation.clone();
            (owner, closes)
        })
        .collect();
    let now = Instant::now();
    for second in 0..300 {
        if (1..=10).contains(&second) {
            workers.lock().unwrap().0 += 1;
        }
        for (owner, _) in &mut sessions {
            owner.poll_idle_retirement(now + Duration::from_secs(second), false);
        }
    }
    assert_eq!(metrics.checks.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.snapshots.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.inbox_checks.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.scheduled.lock().unwrap().len(), 64);
    for (owner, closes) in &mut sessions {
        owner.poll_idle_retirement(now + IDLE_TIMEOUT, false);
        assert_eq!(
            closes.get(),
            0,
            "old timer cannot retire a recently active session"
        );
    }
    assert_eq!(metrics.checks.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.scheduled.lock().unwrap().len(), 128);
    for (owner, closes) in &mut sessions {
        owner.poll_idle_retirement(now + IDLE_TIMEOUT + Duration::from_secs(10), false);
        assert_eq!(closes.get(), 1);
    }
    assert_eq!(metrics.checks.load(Ordering::Relaxed), 64);
    assert_eq!(metrics.snapshots.load(Ordering::Relaxed), 64);
    assert_eq!(metrics.inbox_checks.load(Ordering::Relaxed), 64);
}
