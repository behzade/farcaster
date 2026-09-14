//! Real installed-harness smoke cases.  These stay ignored because each case
//! consumes a model turn; `scripts/e2e.sh` runs one case/harness/process.
//!
//! Do not replace these with fixture transports.  The point is to exercise
//! `spawn_session`, the installed binary, its real model account, normalized
//! activities, transcript projection, and native history together.

use std::{
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{
    agents::{
        SessionOperation,
        extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
    },
    conversation::TranscriptKind,
};

use super::live_tests::support::{
    LiveSession, Submission, TurnGate, alternate_image, bounded_command_permission,
    bounded_gate_permission, for_each_selected, image, marker,
};
use super::live_tests::{TEST_IMAGE, TURN_TIMEOUT};

const BASIC_TIMEOUT: Duration = TURN_TIMEOUT;
const GATE_SCRIPT_TIMEOUT: Duration = Duration::from_secs(5);

struct OwnedGateChild(Child);

impl OwnedGateChild {
    fn spawn(project: &std::path::Path, gate: &TurnGate) -> Result<Self, String> {
        Command::new("/bin/sh")
            .arg(format!("./{}.sh", gate.file_name()))
            .current_dir(project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(Self)
            .map_err(|error| format!("start owned live gate script: {error}"))
    }

    fn wait_for_exit(&mut self) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + GATE_SCRIPT_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self
                .0
                .try_wait()
                .map_err(|error| format!("poll owned live gate script: {error}"))?
            {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err("owned live gate script did not exit within 5 seconds".into())
    }
}

impl Drop for OwnedGateChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn wait_for_gate_process(gate: &TurnGate) -> Result<(), String> {
    let deadline = Instant::now() + GATE_SCRIPT_TIMEOUT;
    while Instant::now() < deadline {
        if gate.has_started() {
            return gate.assert_process_alive();
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("owned live gate script did not write its start witness within 5 seconds".into())
}

#[test]
fn turn_gate_explicit_release_exits_zero() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let gate = TurnGate::new(project.path(), "release")?;
    let mut child = OwnedGateChild::spawn(project.path(), &gate)?;
    wait_for_gate_process(&gate)?;
    gate.release()?;
    let status = child.wait_for_exit()?;
    if status.code() != Some(0) {
        return Err(format!(
            "released live gate script exited as {status:?}, expected 0"
        ));
    }
    Ok(())
}

#[test]
fn turn_gate_project_teardown_exits_125_before_timeout() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let gate = TurnGate::new(project.path(), "teardown")?;
    let mut child = OwnedGateChild::spawn(project.path(), &gate)?;
    wait_for_gate_process(&gate)?;
    project
        .close()
        .map_err(|error| format!("remove owned live gate project: {error}"))?;
    let closed = gate
        .assert_still_closed()
        .expect_err("removed live gate script must not pass a control assertion");
    if !closed.contains("script disappeared") {
        return Err(format!("unexpected removed-gate error: {closed}"));
    }
    let status = child.wait_for_exit()?;
    if status.code() != Some(125) {
        return Err(format!(
            "torn-down live gate script exited as {status:?}, expected 125"
        ));
    }
    Ok(())
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_regular_message_response_once() -> Result<(), String> {
    for_each_selected(|session| regular_response(session).map(|_| ()))
}

fn regular_response(session: &mut LiveSession) -> Result<(Submission, String), String> {
    session.require_available("normal prompts", &session.capabilities().turns.prompt)?;
    let input = marker("normal_input");
    let answer = marker("normal_answer");
    let cursor = session.activity_cursor();
    let submission = session.submit(
        PromptMode::Normal,
        format!(
            "This is a live integration test with input ID {input}. Reply with the exact token {answer} and no other words."
        ),
        Vec::new(),
    )?;
    require_accepted(session, &submission)?;
    session.wait_for_settled_after(cursor, BASIC_TIMEOUT)?;
    session.assert_transcript_user_once(&input, 0)?;
    let answer_count = session
        .conversation()
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::Assistant)
        .map(|item| item.complete_text().matches(answer.as_str()).count())
        .sum::<usize>();
    if answer_count != 1 {
        return Err(format!(
            "expected one completed model response containing {answer:?}, found {answer_count}"
        ));
    }
    Ok((submission, answer))
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_regular_message_response_history_once() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available("native history", &session.capabilities().sessions.history)?;
        let (submission, answer) = regular_response(session)?;
        session.assert_submission_once(&submission)?;
        let history = session.history_reload()?;
        if !history.iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message.to_string().contains(&answer)
        }) {
            return Err(format!(
                "native history omitted the completed assistant response {answer:?}: {history:?}"
            ));
        }
        session.assert_transcript_user_once(&submission.marker, 0)
    })
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_thinking_block_is_exposed_or_blocked() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available(
            "reasoning observation",
            &session.capabilities().observation.reasoning,
        )?;
        let input = marker("thinking_input");
        let answer = marker("thinking_answer");
        let cursor = session.activity_cursor();
        let submission = session.submit(
            PromptMode::Normal,
            format!(
                "Solve 17 * 19 carefully, then reply with the exact token {answer}. This live test input ID is {input}."
            ),
            Vec::new(),
        )?;
        require_accepted(session, &submission)?;
        let observed = session
            .wait_for_activity_after(cursor, BASIC_TIMEOUT, |event| {
                let nonempty_thinking = event.get("type").and_then(Value::as_str)
                    == Some("message_update")
                    && event
                        .pointer("/assistantMessageEvent/type")
                        .and_then(Value::as_str)
                        == Some("thinking_delta")
                    && event
                        .pointer("/assistantMessageEvent/delta")
                        .and_then(Value::as_str)
                        .is_some_and(|delta| !delta.trim().is_empty());
                nonempty_thinking
                    || event.get("type").and_then(Value::as_str) == Some("agent_settled")
            })
            .map_err(|error| {
                format!(
                    "E2E_BLOCKED: {} did not expose a real thinking block: {error}",
                    session.harness()
                )
            })?;
        if observed.get("type").and_then(Value::as_str) == Some("agent_settled") {
            return Err(format!(
                "E2E_BLOCKED: {} settled without a nonempty real thinking_delta; trace={}",
                session.harness(),
                session.trace_summary()
            ));
        }
        if !session.conversation().items.iter().any(|item| {
            item.kind == TranscriptKind::Thinking && !item.complete_text().trim().is_empty()
        }) {
            return Err(format!(
                "{} emitted thinking activity but projected no nonempty thinking transcript row: {}",
                session.harness(),
                session.transcript_summary()
            ));
        }
        session.wait_for_assistant_text(&answer, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(cursor, BASIC_TIMEOUT)
    })
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_tool_lifecycle_and_failed_command_output_are_visible() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available(
            "tool activity",
            &session.capabilities().observation.tool_activity,
        )?;
        let missing = marker("missing_file").to_ascii_lowercase();
        let answer = marker("tool_error_answer");
        let command = session.register_fixture_command(format!("cat {missing}"))?;
        let cursor = session.activity_cursor();
        let submission = session.submit(
            PromptMode::Normal,
            format!(
                "Use the shell tool to run exactly `{command}`. It must fail because that file does not exist. Then reply with the exact token {answer}. Do not create the file."
            ),
            Vec::new(),
        )?;
        require_accepted(session, &submission)?;
        let started = session.wait_for_activity_after(cursor, BASIC_TIMEOUT, |event| {
            event.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                && event.to_string().contains(&missing)
        })?;
        let tool_id = started
            .get("toolCallId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tool start omitted toolCallId: {started}"))?
            .to_owned();
        let ended = session.wait_for_activity_after(cursor, BASIC_TIMEOUT, |event| {
            event.get("type").and_then(Value::as_str) == Some("tool_execution_end")
                && event.get("toolCallId").and_then(Value::as_str) == Some(&tool_id)
        })?;
        let end_wire = ended.to_string();
        let nonzero_exit = [
            "Command exited with code 1",
            "Process exited with code 1",
            "Exit code: 1",
            "exitCode\":1",
            "exit_code\":1",
            "exit:1",
        ]
        .iter()
        .any(|witness| end_wire.contains(witness));
        let native_error = ended.get("isError").and_then(Value::as_bool) == Some(true);
        if !end_wire.contains(&missing) || !(nonzero_exit || native_error) {
            return Err(format!(
                "failed shell command did not preserve its missing path and failure evidence in the real tool result: {ended}"
            ));
        }
        let visible = session
            .conversation()
            .items
            .iter()
            .filter(|item| {
                item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(&tool_id)
            })
            .collect::<Vec<_>>();
        if visible.len() != 1 || visible[0].streaming || !visible[0].tool_output.contains(&missing)
        {
            return Err(format!(
                "failed command result did not stay in one completed visible tool row for {tool_id}: {}",
                session.transcript_summary()
            ));
        }
        eprintln!(
            "E2E_NOTE: this case proves failed-command output; native protocol-error badge coverage is a separate feature"
        );
        session.wait_for_assistant_text(&answer, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(cursor, BASIC_TIMEOUT)
    })
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_mixed_streaming_preserves_tool_and_text_order() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available(
            "streamed text",
            &session.capabilities().observation.streamed_text,
        )?;
        session.require_available(
            "tool activity",
            &session.capabilities().observation.tool_activity,
        )?;
        let tool_token = marker("mixed_tool");
        let answer = marker("mixed_answer");
        let command = session.register_fixture_command(format!("printf {tool_token}"))?;
        let cursor = session.activity_cursor();
        let submission = session.submit(
            PromptMode::Normal,
            format!(
                "Use the shell tool to run exactly `{command}`. After it returns, explain in one short sentence that it printed {tool_token}, then include the exact token {answer}."
            ),
            Vec::new(),
        )?;
        require_accepted(session, &submission)?;
        let tool_start = session.wait_for_activity_after(cursor, BASIC_TIMEOUT, |event| {
            event.get("type").and_then(Value::as_str) == Some("tool_execution_start")
                && event.to_string().contains(&tool_token)
        })?;
        let tool_id = tool_start
            .get("toolCallId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tool start omitted toolCallId: {tool_start}"))?
            .to_owned();
        session.wait_for_activity_after(cursor, BASIC_TIMEOUT, |event| {
            event.get("type").and_then(Value::as_str) == Some("tool_execution_end")
                && event.get("toolCallId").and_then(Value::as_str) == Some(&tool_id)
        })?;
        session.wait_for_assistant_text(&answer, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(cursor, BASIC_TIMEOUT)?;
        let events = session.activities();
        let tool_end = events
            .iter()
            .position(|event| {
                event.value.get("type").and_then(Value::as_str) == Some("tool_execution_end")
                    && event.value.get("toolCallId").and_then(Value::as_str) == Some(&tool_id)
            })
            .ok_or_else(|| "mixed turn lost tool end from event trace".to_owned())?;
        let text_after_tool = events.iter().skip(tool_end + 1).any(|event| {
            event.value.get("type").and_then(Value::as_str) == Some("message_update")
                && event
                    .value
                    .pointer("/assistantMessageEvent/type")
                    .and_then(Value::as_str)
                    == Some("text_delta")
        });
        if !text_after_tool {
            return Err(format!(
                "mixed turn did not emit assistant text after its real tool completed: {}",
                session.trace_summary()
            ));
        }
        let tool_rows = session
            .conversation()
            .items
            .iter()
            .filter(|item| {
                item.kind == TranscriptKind::Tool && item.tool_call_id.as_deref() == Some(&tool_id)
            })
            .count();
        if tool_rows != 1 {
            return Err(format!(
                "expected one visible tool row for {tool_id}, found {tool_rows}"
            ));
        }
        Ok(())
    })
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_images_and_image_only_input_reach_native_history() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available("image prompts", &session.capabilities().turns.images)?;
        let input = marker("image_input");
        let answer = marker("image_answer");
        let cursor = session.activity_cursor();
        let image = image(TEST_IMAGE, "image/png");
        let submission = session.submit(
            PromptMode::Normal,
            format!(
                "The attached image is part of a live integration test {input}. Reply with the exact token {answer}."
            ),
            vec![image.clone()],
        )?;
        require_accepted(session, &submission)?;
        session.wait_for_assistant_text(&answer, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(cursor, BASIC_TIMEOUT)?;
        session.assert_submission_once(&submission)?;

        let image_only_cursor = session.activity_cursor();
        let image_only =
            session.submit(PromptMode::Normal, String::new(), vec![alternate_image()])?;
        require_accepted(session, &image_only)?;
        session.wait_for_settled_after(image_only_cursor, BASIC_TIMEOUT)?;
        let image_only_rows = session
            .conversation()
            .items
            .iter()
            .filter(|item| {
                item.kind == TranscriptKind::User
                    && item.complete_text().trim().is_empty()
                    && item.images.len() == 1
            })
            .count();
        if image_only_rows != 1 {
            return Err(format!(
                "expected exactly one visible image-only user row, found {image_only_rows}: {}",
                session.transcript_summary()
            ));
        }
        session.assert_native_user_once("", &image_only.images).map_err(|error| {
            format!(
                "image-only input must create a real native user record, not merely a local row: {error}"
            )
        })
    })
}

#[test]
#[ignore = "real installed harness/model E2E; run through scripts/e2e.sh"]
fn live_e2e_identical_text_with_distinct_images_keeps_distinct_ids() -> Result<(), String> {
    for_each_selected(|session| {
        session.require_available("image prompts", &session.capabilities().turns.images)?;
        session.require_prompt_delivery_tracking(PromptMode::Normal)?;
        let shared = "Live E2E identical-text receipt test. Reply with OK.";
        let first_cursor = session.activity_cursor();
        let first = session.submit(
            PromptMode::Normal,
            shared,
            vec![image(TEST_IMAGE, "image/png")],
        )?;
        require_accepted(session, &first)?;
        session.wait_for_delivery_after(first_cursor, &first.id, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(first_cursor, BASIC_TIMEOUT)?;

        let second_cursor = session.activity_cursor();
        let second = session.submit(PromptMode::Normal, shared, vec![alternate_image()])?;
        require_accepted(session, &second)?;
        session.wait_for_delivery_after(second_cursor, &second.id, BASIC_TIMEOUT)?;
        session.wait_for_settled_after(second_cursor, BASIC_TIMEOUT)?;
        if first.id == second.id {
            return Err("identical text reused a submission ID".into());
        }
        session.assert_duplicate_submissions_once(&first, &second)
    })
}

fn require_accepted(session: &mut LiveSession, submission: &Submission) -> Result<(), String> {
    let response = session.wait_for_response(&submission.id, BASIC_TIMEOUT)?;
    if response.operation() != SessionOperation::Prompt(submission.mode) {
        return Err(format!(
            "submission {} returned {:?}, expected prompt {:?}",
            submission.id,
            response.operation(),
            submission.mode
        ));
    }
    response
        .result
        .map(|_| ())
        .map_err(|error| format!("submission {} was not accepted: {error}", submission.id))
}

#[test]
fn bounded_gate_permission_allows_only_the_registered_script() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let gate = TurnGate::new(project.path(), "permission")?;
    let request = ExtensionUiRequest::Select {
        id: "gate-permission".into(),
        title: format!(
            "Allow Bash?\n{}",
            serde_json::json!({"command": gate.shell_command(), "description": "live fixture"})
        ),
        options: vec!["Deny".into(), "Allow".into()],
        timeout: None,
    };
    assert_eq!(
        bounded_gate_permission(&request, &[gate]),
        Ok(ExtensionUiResponse::Value {
            id: "gate-permission".into(),
            value: "Allow".into(),
        })
    );
    Ok(())
}

#[test]
fn bounded_command_permission_allows_only_exact_oneshot_acp_forms() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let command = TurnGate::new(project.path(), "permission")?.shell_command();
    let select = |id: &str, title: String, options: Vec<String>| ExtensionUiRequest::Select {
        id: id.into(),
        title,
        options,
        timeout: None,
    };
    let allowed = vec![command.clone()];
    for (request, id, value) in [
        (
            select(
                "cursor-raw",
                command.clone(),
                vec!["Allow once".into(), "Allow always".into(), "Reject".into()],
            ),
            "cursor-raw",
            "Allow once",
        ),
        (
            select(
                "cursor-backtick",
                format!("`{command}`"),
                vec!["Allow once".into(), "Allow always".into(), "Reject".into()],
            ),
            "cursor-backtick",
            "Allow once",
        ),
        (
            select(
                "antigravity",
                command.clone(),
                vec!["Allow Always (risky)".into(), "Allow".into(), "Deny".into()],
            ),
            "antigravity",
            "Allow",
        ),
    ] {
        assert_eq!(
            bounded_command_permission(&request, &allowed),
            Ok(ExtensionUiResponse::Value {
                id: id.into(),
                value: value.into()
            })
        );
    }
    Ok(())
}

#[test]
fn bounded_gate_permission_rejects_unregistered_or_composed_commands() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let gate = TurnGate::new(project.path(), "permission")?;
    let select = |title: String, options: Vec<String>| ExtensionUiRequest::Select {
        id: "gate-permission".into(),
        title,
        options,
        timeout: None,
    };
    let exact = gate.shell_command();
    let invalid = [
        select(
            format!(
                "Allow Bash?\n{}",
                serde_json::json!({"command": format!("{exact}; touch not-allowed")})
            ),
            vec!["Deny".into(), "Allow".into()],
        ),
        select(
            format!(
                "Allow Bash?\n{}",
                serde_json::json!({"command": format!("{exact} extra-path")})
            ),
            vec!["Deny".into(), "Allow".into()],
        ),
        select(
            "Allow Bash?\n{\"command\":\"sh ./not-a-registered-gate.sh\"}".into(),
            vec!["Deny".into(), "Allow".into()],
        ),
        select(
            format!("Allow Bash?\n{}", serde_json::json!({"command": exact})),
            vec!["Allow".into(), "Deny".into()],
        ),
    ];
    for request in invalid {
        let error = bounded_gate_permission(&request, std::slice::from_ref(&gate))
            .expect_err("unsafe Bash selection must not receive an Allow response");
        assert!(error.contains("E2E_BLOCKED"), "{error}");
    }
    let confirm = ExtensionUiRequest::Confirm {
        id: "gate-permission".into(),
        title: "Allow deleting unrelated data?".into(),
        message: "no".into(),
        timeout: None,
    };
    assert!(
        bounded_gate_permission(&confirm, &[gate])
            .expect_err("non-gate dialog must not receive an approval")
            .contains("E2E_BLOCKED")
    );
    Ok(())
}

#[test]
fn bounded_command_permission_rejects_nonexact_acp_titles_and_choices() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let command = TurnGate::new(project.path(), "permission")?.shell_command();
    let select = |title: String, options: Vec<String>| ExtensionUiRequest::Select {
        id: "gate-permission".into(),
        title,
        options,
        timeout: None,
    };
    let invalid = [
        // Antigravity's observed form is raw only: it must not accept a
        // wrapper, suffix, or a different option order.
        select(
            format!("`{command}`"),
            vec!["Allow Always (risky)".into(), "Allow".into(), "Deny".into()],
        ),
        select(
            format!("{command}; touch forbidden"),
            vec!["Allow Always (risky)".into(), "Allow".into(), "Deny".into()],
        ),
        // Cursor may present exactly one enclosing pair of backticks, but no
        // prose, nesting, or changed choice order.
        select(
            format!("run `{command}` now"),
            vec!["Allow once".into(), "Allow always".into(), "Reject".into()],
        ),
        select(
            format!("``{command}``"),
            vec!["Allow once".into(), "Allow always".into(), "Reject".into()],
        ),
        select(
            command.clone(),
            vec!["Allow always".into(), "Allow once".into(), "Reject".into()],
        ),
    ];
    for request in invalid {
        let error = bounded_command_permission(&request, std::slice::from_ref(&command))
            .expect_err("nonexact ACP command selection must not receive an approval");
        assert!(error.contains("E2E_BLOCKED"), "{error}");
    }
    Ok(())
}
