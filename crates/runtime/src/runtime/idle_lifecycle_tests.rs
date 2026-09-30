use super::*;
use crate::runtime::idle::IDLE_TIMEOUT;

#[test]
fn idle_child_report_resumes_once_and_survives_failed_startup() {
    isolated_title(
        "idle_lifecycle_tests::idle_child_report_resumes_once_and_survives_failed_startup",
        || {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir("inner").unwrap();
            std::os::unix::fs::symlink("../control.sock", "inner/control.sock").unwrap();
            fs::rename("pi", "inner/pi").unwrap();
            fs::write("pi", "#!/bin/sh\nprintf '%s' \"$FARCASTER_MCP_CALLER\" > caller-token\nexec \"$PWD/inner/pi\" \"$@\"\n").unwrap();
            fs::set_permissions("pi", fs::Permissions::from_mode(0o755)).unwrap();
            let mut scenario = Scenario::new(Backend::Pi, Some("Saved chat"), true);
            let registry = agents::CallerRegistry::shared();
            let token = fs::read_to_string("caller-token").unwrap();
            let parent = registry.resolve(&token).unwrap();
            let (responses, response) = mpsc::channel();
            let _approval = registry
                .request_profile_input(
                    &parent,
                    agents::WorkerInput {
                        id: "unanswered-approval".into(),
                        prompt: "Allow child?".into(),
                        options: vec![],
                        secret: false,
                    },
                    responses,
                )
                .unwrap();
            scenario.owner.publish_child_inputs();
            scenario.pump();
            let approval_id = scenario
                .events
                .iter()
                .find_map(|event| match event {
                    RuntimeEvent::ExtensionUi { request, .. } => {
                        request.dialog_id().map(str::to_owned)
                    }
                    _ => None,
                })
                .unwrap();
            let assert_replayed = |scenario: &Scenario| {
                let generation = scenario.owner.process_generation;
                let reset = scenario.events.iter().rposition(|event| matches!(event,
                    RuntimeEvent::SessionReset { generation: current, .. } if *current == generation
                )).unwrap();
                let approvals: Vec<_> = scenario.events.iter().enumerate().filter(|(_, event)| matches!(event,
                    RuntimeEvent::ExtensionUi { generation: current, request, .. }
                    if *current == generation && request.dialog_id() == Some(approval_id.as_str())
                )).collect();
                assert_eq!(
                    approvals.len(),
                    1,
                    "unanswered approval must replay once after reset"
                );
                assert!(approvals[0].0 > reset);
            };
            let child = registry
                .issue_as_with_access(
                    &parent.project,
                    agents::CallerProfile {
                        backend: Backend::Pi,
                        provider: None,
                        model: None,
                        effort: None,
                    },
                    None,
                    "reporter-id".into(),
                    "reporter".into(),
                    Some(parent.worker_id),
                    agents::HarnessAccessMode::Auto,
                )
                .unwrap();
            child.bind("reporter-session");
            for (index, fail_startup) in [false, true].into_iter().enumerate() {
                scenario.until(|s| {
                    s.owner.idle_retirement.requests.is_empty()
                        && !s.owner.active_snapshot().conversation.running
                });
                scenario.owner.poll_idle_retirement(Instant::now(), false);
                scenario.owner.apply_command(RuntimeCommand::SystemWake);
                assert!(scenario.owner.process.is_none());
                if fail_startup {
                    fs::rename("pi", "pi-disabled").unwrap();
                }
                registry
                    .send(child.token(), "parent", format!("report {index}"))
                    .unwrap();
                scenario.owner.poll_idle_retirement(Instant::now(), false);
                scenario.pump();
                assert_replayed(&scenario);
                if fail_startup {
                    assert!(scenario.owner.process.is_none());
                    assert!(
                        scenario
                            .owner
                            .idle_retirement
                            .inbox
                            .as_ref()
                            .unwrap()
                            .has_pending_messages()
                    );
                    let generation = scenario.owner.process_generation;
                    scenario.owner.poll_idle_retirement(Instant::now(), false);
                    assert_eq!(
                        scenario.owner.process_generation, generation,
                        "must not auto-retry failed startup"
                    );
                    fs::rename("pi-disabled", "pi").unwrap();
                    scenario.owner.apply_command(RuntimeCommand::SetSessionName(
                        "Retried after failed resume".into(),
                    ));
                    scenario.until(|s| {
                        s.backend.state.lock().unwrap().name.as_deref()
                            == Some("Retried after failed resume")
                    });
                    assert_eq!(scenario.backend.rename_requests().len(), 1);
                }
                scenario.until(|s| {
                    s.backend
                        .state
                        .lock()
                        .unwrap()
                        .requests
                        .iter()
                        .filter(|r| r["type"] == "prompt")
                        .count()
                        == index + 1
                        && !s.owner.active_snapshot().conversation.running
                });
                scenario.owner.publish_child_inputs();
                scenario.pump();
                assert_replayed(&scenario);
                assert!(
                    scenario
                        .owner
                        .idle_retirement
                        .inbox
                        .as_ref()
                        .unwrap()
                        .transferred()
                );
            }
            let requests = &scenario.backend.state.lock().unwrap().requests;
            let prompts: Vec<_> = requests.iter().filter(|r| r["type"] == "prompt").collect();
            assert_eq!(prompts.len(), 2);
            assert!(prompts[0]["message"].as_str().unwrap().contains("report 0"));
            assert!(prompts[1]["message"].as_str().unwrap().contains("report 1"));
            registry
                .respond_to_child_input(agents::WorkerInputResponse {
                    id: approval_id,
                    value: Some("allow".into()),
                    cancel: false,
                })
                .unwrap();
            assert_eq!(response.try_recv().unwrap().id, "unanswered-approval");
        },
    );
}

#[test]
fn idle_retirement_resumes_saved_session_and_delivers_next_prompt_once() {
    isolated_title(
        "idle_lifecycle_tests::idle_retirement_resumes_saved_session_and_delivers_next_prompt_once",
        || {
            for backend in [Backend::Codex, Backend::Pi] {
                let mut scenario = Scenario::new(backend, Some("Saved chat"), true);
                scenario.prompt();
                scenario.until(|s| {
                    !s.owner.normal_prompt_in_flight
                        && !s.owner.active_snapshot().conversation.running
                        && s.owner.idle_retirement.requests.is_empty()
                });
                let path = scenario.owner.active_session.clone();
                let transcript = scenario.owner.snapshot.conversation.clone();
                let stats = json!({
                    "tokens": {"input": 100, "output": 20, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 120},
                    "contextUsage": {"tokens": 120, "contextWindow": 1000, "percent": 12.0}
                });
                scenario.owner.snapshot.stats = stats.clone();
                scenario.owner.snapshot.account_usage.weekly = Some(agents::AccountUsageWindow {
                    remaining_percent: 68.0,
                    resets_at: None,
                });
                let account_usage = scenario.owner.snapshot.account_usage.clone();
                let now = Instant::now();
                scenario.owner.poll_idle_retirement(now, false);
                assert!(
                    scenario.owner.idle_retirement.deadline.is_some(),
                    "{backend} must become eligible"
                );
                scenario
                    .owner
                    .poll_idle_retirement(now + IDLE_TIMEOUT, false);
                assert!(scenario.owner.process.is_none(), "{backend} must retire");
                assert_eq!(transcript, scenario.owner.snapshot.conversation);
                assert_eq!(scenario.owner.active_session, path);
                scenario.owner.start_process(path.clone());
                assert!(scenario.owner.snapshot.document_ready);
                assert_eq!(scenario.owner.snapshot.stats, stats);
                assert_eq!(scenario.owner.snapshot.account_usage, account_usage);
                assert!(Arc::ptr_eq(
                    &scenario.owner.snapshot.conversation,
                    &transcript
                ));
                scenario.until(|s| {
                    s.owner.startup_state_loaded
                        && s.owner.startup_history_loaded
                        && s.owner.idle_retirement.requests.is_empty()
                });
                assert_eq!(scenario.owner.snapshot.stats, stats);
                scenario.prompt();
                scenario.until(|s| {
                    !s.owner.normal_prompt_in_flight
                        && !s.owner.active_snapshot().conversation.running
                });
                assert!(scenario.owner.process.is_some());
                assert_eq!(scenario.owner.active_session, path);
                let state = scenario.backend.state.lock().expect("fixture state");
                assert_eq!(
                    state.title_requests, 0,
                    "resume must not create a new session"
                );
                let prompt_method = match backend {
                    Backend::Codex => "turn/start",
                    Backend::Pi => "prompt",
                    _ => "session/prompt",
                };
                assert_eq!(
                    state
                        .requests
                        .iter()
                        .filter(|r| {
                            r["method"].as_str().or(r["type"].as_str()) == Some(prompt_method)
                        })
                        .count(),
                    2,
                    "{backend}: each prompt must be sent once"
                );
                if backend == Backend::Codex {
                    let resumes: Vec<_> = state
                        .requests
                        .iter()
                        .filter(|r| r["method"] == "thread/resume")
                        .collect();
                    assert_eq!(resumes.len(), 2);
                    assert!(
                        resumes
                            .iter()
                            .all(|r| r["params"]["threadId"] == "main-thread")
                    );
                }
                drop(state);
                let account_usage = scenario.owner.snapshot.account_usage.clone();
                scenario.owner.snapshot.stats = stats;
                scenario.owner.start_process(None);
                assert!(!scenario.owner.snapshot.document_ready);
                assert_eq!(scenario.owner.snapshot.stats, Value::Null);
                assert_eq!(scenario.owner.snapshot.account_usage, account_usage);
            }
        },
    );
}

#[test]
fn renaming_a_retired_session_resumes_and_sends_the_command_once() {
    isolated_title(
        "idle_lifecycle_tests::renaming_a_retired_session_resumes_and_sends_the_command_once",
        || {
            for backend in [Backend::Codex, Backend::Pi] {
                let mut scenario = Scenario::new(backend, Some("Saved chat"), true);
                scenario.until(|s| s.owner.idle_retirement.requests.is_empty());
                scenario.owner.apply_command(RuntimeCommand::SystemWake);
                assert!(scenario.owner.process.is_none());
                let executable = match backend {
                    Backend::Codex => "codex",
                    Backend::Pi => "pi",
                    _ => unreachable!(),
                };
                fs::rename(executable, "disabled-harness").unwrap();
                scenario
                    .owner
                    .apply_command(RuntimeCommand::SetSessionName("Failed rename".into()));
                assert!(scenario.owner.process.is_none());
                assert!(scenario.backend.rename_requests().is_empty());
                fs::rename("disabled-harness", executable).unwrap();
                scenario
                    .owner
                    .apply_command(RuntimeCommand::SetSessionName("Renamed after idle".into()));
                scenario.until(|s| {
                    s.backend.state.lock().unwrap().name.as_deref() == Some("Renamed after idle")
                });
                assert_eq!(scenario.backend.rename_requests().len(), 1);
            }
        },
    );
}
