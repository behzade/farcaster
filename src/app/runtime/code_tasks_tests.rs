use super::*;

const PROMPT: &str = "Fix this\n\nCode context: unsaved.rs:2:1\n```\nunsaved code\n```";

fn ready_original() -> Harness {
    let harness = Harness::start();
    harness.select("claude", "original");
    let mut catalog = harness.accept(WAIT).expect("catalog starts");
    catalog.complete_catalog(false);
    harness.snapshot("claude", |s| {
        s.configuration_status == ConfigurationStatus::Loaded
    });
    while harness.runtime.try_recv().is_ok() {}
    harness
}

fn task(harness: &Harness, id: &str, model: Option<Model>) -> RuntimeCommand {
    RuntimeCommand::StartTask {
        id: id.into(),
        settings: TaskSettings {
            project: harness.project.clone(),
            harness: "claude".into(),
            model,
            effort: None,
            access_mode: HarnessAccessMode::Sandboxed,
        },
        message: PROMPT.into(),
    }
}

fn result(harness: &Harness, id: &str, background: bool) -> (bool, Option<PathBuf>) {
    let deadline = Instant::now() + WAIT;
    loop {
        match harness.runtime.try_recv() {
            Ok(RuntimeEvent::SessionReset { .. } | RuntimeEvent::HistoryReset { .. })
                if background =>
            {
                panic!("background task changed selection");
            }
            Ok(RuntimeEvent::Snapshot { snapshot, .. }) if background => {
                assert!(
                    snapshot.conversation.items.is_empty(),
                    "task reached original chat"
                );
            }
            Ok(RuntimeEvent::PromptResult {
                target,
                accepted,
                session,
            }) => {
                assert_eq!(target, format!("draft:{id}"));
                return (accepted, session);
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "no task result");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn starts_without_selecting_its_chat() {
    isolated(
        "code_tasks_tests::starts_without_selecting_its_chat",
        &["claude"],
        || {
            let harness = ready_original();
            let model = serde_json::from_value(json!({
                "id":"fixture-model", "name":"Fixture model", "provider":"claude"
            }))
            .expect("model");
            let command = task(&harness, "task", Some(model));
            harness.runtime.send(command.clone()).expect("start task");
            harness.runtime.send(command).expect("duplicate request");
            let mut peer = harness.accept(WAIT).expect("task starts");
            peer.complete_catalog(false);
            let model = peer.request("set_model");
            assert_eq!(model["request"]["model"], "fixture-model");
            peer.reply(&model, json!({}));
            let mut line = String::new();
            peer.reader.read_line(&mut line).expect("read prompt");
            let prompt: Value = serde_json::from_str(&line).expect("prompt JSON");
            assert_eq!(prompt["type"], "user");
            assert_eq!(prompt["message"]["content"][0]["text"], PROMPT);
            peer.write(prompt);
            let (accepted, session) = result(&harness, "task", true);
            assert!(accepted);
            assert!(
                harness.accept(Duration::from_millis(50)).is_none(),
                "duplicate launched a process"
            );
            harness
                .runtime
                .send(RuntimeCommand::SelectSession {
                    path: session.expect("accepted task has a session"),
                    harness: "claude".into(),
                    session_id: String::new(),
                    project: harness.project.clone(),
                })
                .expect("open task chat");
            harness.snapshot("claude", |s| !s.conversation.items.is_empty());
        },
    );
}

#[test]
fn failure_leaves_original_selected() {
    isolated(
        "code_tasks_tests::failure_leaves_original_selected",
        &["claude"],
        || {
            let harness = ready_original();
            harness
                .runtime
                .send(task(&harness, "failed", None))
                .expect("start task");
            harness
                .accept(WAIT)
                .expect("task starts")
                .complete_catalog(true);
            assert!(!result(&harness, "failed", true).0);
        },
    );
}

#[test]
fn opening_during_startup_does_not_restart_task() {
    isolated(
        "code_tasks_tests::opening_during_startup_does_not_restart_task",
        &["claude"],
        || {
            let harness = Harness::start();
            harness
                .runtime
                .send(task(&harness, "early", None))
                .expect("start task");
            // Select before the actor can finish startup or publish a locator.
            harness.select("claude", "early");
            let mut peer = harness.accept(WAIT).expect("task starts");
            // Selecting also loads a catalog; either process may connect first.
            let mut catalog = harness.accept(WAIT).expect("catalog starts");
            peer.complete_catalog(false);
            catalog.complete_catalog(false);
            let mut prompt = String::new();
            if peer.reader.read_line(&mut prompt).expect("read task") == 0 {
                catalog
                    .reader
                    .read_line(&mut prompt)
                    .expect("read other peer");
                peer = catalog;
            }
            let prompt: Value = serde_json::from_str(&prompt).expect("task prompt");
            assert_eq!(prompt["type"], "user");
            peer.write(prompt);
            assert!(result(&harness, "early", false).0);
            assert!(
                harness.accept(Duration::from_millis(50)).is_none(),
                "opening restarted the process"
            );
        },
    );
}
