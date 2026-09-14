//! Opt-in checks of the installed harness through the actual Farcaster app.
//!
//! `scripts/e2e.sh` creates `FARCASTER_DATA_DIR`; this file refuses to use a
//! normal user state directory. It sends real model requests and is ignored by
//! ordinary test runs.
use crate::agents::Backend;

use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use gpui::{
    AnyWindowHandle, InputEvent as _, KeyDownEvent, Keystroke, Modifiers, TestAppContext,
    VisualTestContext,
};

use super::{
    FarcasterApp, infrastructure::persistence::StateStore, ui::theme::install_component_theme,
};
use crate::{
    agents::{
        HarnessAccessMode,
        live_e2e_support::{self, TURN_TIMEOUT, TurnGate, new_turn_gate},
    },
    conversation::{ConversationState, TranscriptItem, TranscriptKind},
    protocol::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
    runtime::ConfigurationStatus,
};

const UI_POLL: Duration = Duration::from_millis(20);
const QUEUE_VISIBILITY_TIMEOUT: Duration = Duration::from_secs(5);
// Pi's real catalog process has a 15s readiness deadline and a second 15s
// model/reasoning deadline. Keep the app fixture bounded just above that
// concrete production path rather than spending a full turn timeout on an
// unavailable configuration.
const CONFIGURATION_TIMEOUT: Duration = Duration::from_secs(35);

fn phase(name: &str) {
    // The ignored suite is intentionally verbose. A live harness can block on
    // native startup or a permission prompt, and the case log must identify
    // that phase instead of looking like a test-runner deadlock.
    eprintln!("LIVE_UI_PHASE {name}");
}

/// Releases a real shell gate even if an assertion fails while its model turn
/// is pending. The gate itself only writes inside this test's temp project.
struct ReleaseGate(TurnGate);

impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let _ = self.0.release();
    }
}

#[derive(Clone, Debug)]
struct LiveUiConfig {
    harness: Backend,
    model: Option<String>,
    access_mode: HarnessAccessMode,
}

fn live_config() -> Result<LiveUiConfig, String> {
    let harness = std::env::var("FARCASTER_E2E_HARNESS")
        .map_err(|_| "live app E2E requires FARCASTER_E2E_HARNESS".to_owned())?;
    let selected = live_e2e_support::selected_live_harnesses()?;
    if selected.len() != 1 || selected.first().copied() != Some(harness.as_str()) {
        return Err(format!(
            "FARCASTER_E2E_HARNESS {harness:?} did not select exactly one installed harness"
        ));
    }
    // Match the shell runner's isolation rule exactly. This prevents an
    // ignored test invoked by hand from touching the user's normal database.
    live_e2e_support::e2e_case_dir()?;
    Ok(LiveUiConfig {
        access_mode: live_e2e_support::live_access_mode_for_harness(harness.parse()?)?,
        harness: harness.parse()?,
        model: std::env::var("FARCASTER_E2E_MODEL")
            .ok()
            .filter(|model| !model.trim().is_empty()),
    })
}

fn wait_for(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    description: &str,
    predicate: impl Fn(&FarcasterApp) -> bool,
) -> Result<(), String> {
    wait_for_with_timeout(cx, app, description, TURN_TIMEOUT, predicate)
}

fn wait_for_with_timeout(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    description: &str,
    timeout: Duration,
    predicate: impl Fn(&FarcasterApp) -> bool,
) -> Result<(), String> {
    phase(&format!("wait:{description}"));
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // This is the app's real runtime projection. It drains only events
        // emitted by the installed session; the test never creates events.
        let complete = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            // Runtime projection can resolve a submission during the next
            // render. Draw one real frame, but never call GPUI's unbounded
            // test-executor drain: Farcaster owns permanent receiver tasks.
            window.draw(cx).clear(cx);
            predicate(app.read(cx))
        });
        if complete {
            phase(&format!("observed:{description}"));
            return Ok(());
        }
        thread::sleep(UI_POLL);
    }
    let snapshot = cx.update(|_, cx| app.read(cx).snapshot.clone());
    Err(format!(
        "timed out after {} seconds waiting for {description}; status={} harness={} transcript={}",
        timeout.as_secs(),
        snapshot.status,
        snapshot.harness.map(Backend::as_str).unwrap_or(""),
        transcript_summary(&snapshot.conversation),
    ))
}

fn submission_diagnostics(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    label: &str,
    markers: &[&str],
) {
    let details = cx.update(|_, cx| {
        let app = app.read(cx);
        let target = app.composer_sessions.current_target();
        let visible_queue = visible_prompt_queue(app);
        let composer = app.composer.read(cx).value().to_owned();
        let rows = markers
            .iter()
            .map(|marker| user_rows(app, marker).len())
            .collect::<Vec<_>>();
        format!(
            "composer={composer:?} can_submit={} pending={} running={} status={} steering={:?} follow_up={:?} user_rows={rows:?}",
            app.can_submit(),
            app.pending_submissions.values().any(|pending|
                pending.submitted_target == target && pending.result.is_none()
            ),
            app.snapshot.conversation.running,
            app.snapshot.status,
            visible_queue.steering,
            visible_queue.follow_up,
        )
    });
    phase(&format!("{label}:{details}"));
}

fn transcript_summary(conversation: &ConversationState) -> String {
    conversation
        .items
        .iter()
        .map(|item| format!("{:?}:{}", item.kind, item.complete_text()))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn user_rows(app: &FarcasterApp, marker: &str) -> Vec<std::sync::Arc<TranscriptItem>> {
    app.snapshot
        .conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User && item.complete_text().contains(marker))
        .cloned()
        .collect()
}

fn queued_steer(app: &FarcasterApp, marker: &str) -> bool {
    visible_prompt_queue(app)
        .steering
        .iter()
        .any(|message| message.contains(marker))
}

fn queued_follow_up(app: &FarcasterApp, marker: &str) -> bool {
    visible_prompt_queue(app)
        .follow_up
        .iter()
        .any(|message| message.contains(marker))
}

fn visible_prompt_queue(app: &FarcasterApp) -> crate::conversation::QueueState {
    crate::app::composer::submissions::visible_prompt_queue(
        &app.snapshot.conversation.queue,
        &app.pending_submissions,
        app.composer_sessions.current_target(),
    )
}

fn has_exact_queued_inputs(
    app: &FarcasterApp,
    steering: Option<&str>,
    follow_up: Option<&str>,
) -> bool {
    let queue = visible_prompt_queue(app);
    let matches = |messages: &[String], marker: Option<&str>| match marker {
        Some(marker) => messages.len() == 1 && messages[0].contains(marker),
        None => messages.is_empty(),
    };
    matches(&queue.steering, steering) && matches(&queue.follow_up, follow_up)
}

fn current_pending_submission_is(app: &FarcasterApp, text: &str) -> bool {
    app.pending_submissions.values().any(|pending| {
        pending.submitted_target == app.composer_sessions.current_target()
            && pending.result.is_none()
            && pending.text.trim_end() == text
    })
}

fn exact_pending_submission(
    app: &FarcasterApp,
    mode: PromptMode,
    submitted_text: &str,
) -> Option<(String, String)> {
    let target = app.composer_sessions.current_target();
    let mut matches = app.pending_submissions.values().filter(|pending| {
        pending.submitted_target == target
            && pending.mode == mode
            && pending.text.trim_end() == submitted_text
            && pending.result.is_none()
    });
    let pending = matches.next()?;
    let exact = (pending.id.clone(), pending.text.clone());
    matches.next().is_none().then_some(exact)
}

/// A mixed steer/follow-up test needs a real rendered queue before it presses
/// Escape. `can_submit` only releases the UI slot for the next real Tab key;
/// it never stands in for acknowledgement or native delivery evidence.
fn wait_for_exact_queue(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    description: &str,
    steering: Option<&str>,
    follow_up: Option<&str>,
    need_next_submit_slot: bool,
) -> Result<(), String> {
    phase(&format!("wait:{description}"));
    let deadline = Instant::now() + QUEUE_VISIBILITY_TIMEOUT;
    let mut last = String::new();
    while Instant::now() < deadline {
        let (queue_matches, can_submit, detail) = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            let app = app.read(cx);
            let visible_queue = visible_prompt_queue(app);
            (
                has_exact_queued_inputs(app, steering, follow_up),
                app.can_submit(),
                format!(
                    "steering={:?} follow_up={:?} pending={} can_submit={}",
                    visible_queue.steering,
                    visible_queue.follow_up,
                    app.pending_submissions
                        .values()
                        .any(|pending| pending.submitted_target
                            == app.composer_sessions.current_target()
                            && pending.result.is_none()),
                    app.can_submit(),
                ),
            )
        });
        last = detail;
        if queue_matches && (!need_next_submit_slot || can_submit) {
            phase(&format!("observed:{description}"));
            return Ok(());
        }
        thread::sleep(UI_POLL);
    }
    let slot = need_next_submit_slot
        .then_some(" and release the composer for the next real Tab submission")
        .unwrap_or_default();
    Err(format!(
        "timed out after {} seconds waiting for {description} to render exact queued inputs{slot}; last state: {last}",
        QUEUE_VISIBILITY_TIMEOUT.as_secs()
    ))
}

/// The held-key case intentionally dispatches its first Escape before it has
/// required an acknowledgement. A pending submission with its exact typed
/// payload, or the real rendered steer queue, is enough proof it reached the
/// app command path; neither is a transcript receipt.
fn wait_for_pending_or_visible_steer(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    text: &str,
    marker: &str,
) -> Result<(), String> {
    phase("wait:held steer pending-or-visible");
    let deadline = Instant::now() + QUEUE_VISIBILITY_TIMEOUT;
    while Instant::now() < deadline {
        let witnessed = cx.update(|window, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            window.draw(cx).clear(cx);
            let app = app.read(cx);
            current_pending_submission_is(app, text)
                || has_exact_queued_inputs(app, Some(marker), None)
        });
        if witnessed {
            phase("observed:held steer pending-or-visible");
            return Ok(());
        }
        thread::sleep(UI_POLL);
    }
    Err(format!(
        "timed out after {} seconds waiting for the held-Escape steer to reach the current pending submission or visible queue",
        QUEUE_VISIBILITY_TIMEOUT.as_secs()
    ))
}

/// A user row is not receipt evidence by itself: an admission path can
/// incorrectly project one. During an Abort race, retain a row only when the
/// live transcript has the matching model effect. Otherwise this UI surface
/// cannot tell a native delivery from an acknowledgement, so fail rather than
/// bless it.
fn delivered_abort_receipts(
    app: &FarcasterApp,
    marker: &str,
) -> Result<Vec<(String, String)>, String> {
    let rows = user_rows(app, marker);
    if rows.len() > 1 {
        return Err(format!(
            "expected at most one {marker} receipt after Abort, found {}",
            rows.len()
        ));
    }
    if rows.is_empty() {
        if assistant_contains(app, marker) {
            return Err(format!(
                "{marker} produced a model effect after Abort without its delivered user transcript row"
            ));
        }
        return Ok(Vec::new());
    }
    if !rows[0].label.is_empty() {
        return Err(format!(
            "E2E_BLOCKED: {marker} became a delivery-unknown transcript row after Abort; this UI snapshot cannot prove whether it was delivered"
        ));
    }
    if !assistant_contains(app, marker) {
        return Err(format!(
            "E2E_BLOCKED: {marker} has an admitted transcript row after Abort, but no native delivery marker or model effect; refusing to count acknowledgement as delivery"
        ));
    }
    Ok(rows
        .into_iter()
        .map(|item| (item.complete_text(), item.label.clone()))
        .collect())
}

fn assistant_contains(app: &FarcasterApp, marker: &str) -> bool {
    !assistant_rows(app, marker).is_empty()
}

fn assistant_rows(app: &FarcasterApp, marker: &str) -> Vec<std::sync::Arc<TranscriptItem>> {
    app.snapshot
        .conversation
        .items
        .iter()
        .filter(|item| {
            item.kind == TranscriptKind::Assistant && item.complete_text().contains(marker)
        })
        .cloned()
        .collect()
}

fn has_gate_tool(app: &FarcasterApp, gate: &TurnGate) -> bool {
    app.snapshot.conversation.items.iter().any(|item| {
        item.kind == TranscriptKind::Tool && item.complete_text().contains(gate.file_name())
    })
}

fn gate_tool_finished(app: &FarcasterApp, gate: &TurnGate) -> bool {
    app.snapshot.conversation.items.iter().any(|item| {
        item.kind == TranscriptKind::Tool
            && item.complete_text().contains(gate.file_name())
            && !item.streaming
    })
}

fn apply_handoff_reached_boundary(
    app: &FarcasterApp,
    gate: &TurnGate,
    completed_runs_before_apply: usize,
) -> bool {
    // ApplySteering has two valid native shapes. A harness can end the exact
    // gated tool in its outer turn, or settle that turn and begin a replacement
    // before the queued work reacts. The latter cannot use tool completion:
    // Codex leaves a managed tool visible while the replacement turn runs.
    let completed_runs = app.snapshot.conversation.completed_runs.len();
    gate_tool_finished(app, gate)
        || (completed_runs > completed_runs_before_apply
            && (app.snapshot.conversation.running
                // A fast replacement can finish between UI polls. Two new
                // completed runs prove the old turn settled and its
                // replacement both started and settled.
                || completed_runs >= completed_runs_before_apply.saturating_add(2)))
}

/// The shared matcher validates the exact registered command and a harness's
/// known one-shot Allow options. This UI layer only turns its selected value
/// into the actual rendered number key; it never sends a response itself.
fn gate_permission_option_index(
    request: &ExtensionUiRequest,
    gate: &TurnGate,
) -> Result<usize, String> {
    let response = live_e2e_support::bounded_command_permission(request, &[gate.shell_command()])?;
    let ExtensionUiResponse::Value { value, .. } = response else {
        return Err("E2E_BLOCKED: gate permission matcher returned a non-select response".into());
    };
    if value.to_ascii_lowercase().contains("always") {
        return Err(format!(
            "E2E_BLOCKED: refusing non-one-shot gate permission value {value:?}"
        ));
    }
    let ExtensionUiRequest::Select { options, .. } = request else {
        return Err("E2E_BLOCKED: gate permission matcher accepted a non-select request".into());
    };
    let index = options.iter().position(|option| option == &value).ok_or_else(|| {
        format!(
            "E2E_BLOCKED: selected gate permission value {value:?} is absent from rendered options {options:?}"
        )
    })?;
    if index >= 5 {
        return Err(format!(
            "E2E_BLOCKED: gate permission option {value:?} is at unsupported rendered shortcut index {}",
            index + 1
        ));
    }
    Ok(index)
}

#[test]
fn gate_permission_matcher_allows_only_the_exact_owned_command() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| format!("create gate project: {error}"))?;
    let gate = new_turn_gate(project.path(), "approval")?;
    let command = gate.shell_command();
    let exact_title = format!("Allow Bash?\n{{\"command\":{command:?}}}");
    let select = |title, options| ExtensionUiRequest::Select {
        id: "gate-permission".into(),
        title,
        options,
        timeout: None,
    };

    // This is the title/options shape captured from Claude's actual gate
    // request. The matcher must select the rendered Allow option, not reply.
    assert_eq!(
        gate_permission_option_index(
            &select(exact_title.clone(), vec!["Deny".into(), "Allow".into()]),
            &gate,
        )?,
        1
    );
    // Cursor renders its safe one-shot option first and wraps only the exact
    // command in one backtick pair. Antigravity/ACP renders Allow second.
    // Keep these indices explicit: the live UI must press the matching
    // rendered number, never a hard-coded "2" or an Always variant.
    assert_eq!(
        gate_permission_option_index(
            &select(
                format!("`{command}`"),
                vec!["Allow once".into(), "Allow always".into(), "Reject".into()],
            ),
            &gate,
        )?,
        0
    );
    assert_eq!(
        gate_permission_option_index(
            &select(
                command.clone(),
                vec!["Allow Always (risky)".into(), "Allow".into(), "Deny".into()],
            ),
            &gate,
        )?,
        1
    );

    let cases = [
        (
            "compound command",
            format!(
                "Allow Bash?\n{{\"command\":{compound:?}}}",
                compound = format!("{command}; touch escaped")
            ),
            vec!["Deny".into(), "Allow".into()],
        ),
        (
            "script only in description",
            format!("Allow Bash?\n{{\"description\":{command:?}}}"),
            vec!["Deny".into(), "Allow".into()],
        ),
        (
            "wrong options",
            exact_title.clone(),
            vec!["Deny".into(), "Allow later".into()],
        ),
        (
            "wrong gate file",
            "Allow Bash?\n{\"command\":\"sh ./another-gate.sh\"}".into(),
            vec!["Deny".into(), "Allow".into()],
        ),
    ];
    for (name, title, options) in cases {
        let error = gate_permission_option_index(&select(title, options), &gate)
            .expect_err(&format!("matcher approved {name}"));
        assert!(
            error.starts_with("E2E_BLOCKED:"),
            "{name} did not report a scoped block: {error}"
        );
    }
    Ok(())
}

fn wait_for_gate_tool(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    gate: &TurnGate,
) -> Result<(), String> {
    phase(&format!("wait:gate-tool:{}", gate.file_name()));
    let deadline = Instant::now() + TURN_TIMEOUT;
    while Instant::now() < deadline {
        let (connected, tool_seen, dialog) = cx.update(|_, cx| {
            app.update(cx, |app, cx| app.drain_runtime(cx));
            let app = app.read(cx);
            (
                app.snapshot.connected,
                has_gate_tool(app, gate),
                app.extension.dialog.clone(),
            )
        });
        // A projected tool row can precede the shell process. Do not let the
        // Escape controls race that start: the gate writes this witness from
        // inside its actual command immediately before entering the loop.
        if connected && tool_seen && gate.has_started() {
            gate.assert_started()?;
            phase(&format!("observed:gate-tool:{}", gate.file_name()));
            return Ok(());
        }
        if dialog.is_none() {
            thread::sleep(UI_POLL);
            continue;
        }
        let request = dialog.as_ref().ok_or_else(|| {
            "E2E_BLOCKED: gate permission disappeared before the rendered choice".to_owned()
        })?;
        let approval_key = (gate_permission_option_index(request, gate)? + 1).to_string();
        // Exercise the rendered dialog's actual keyboard path. Do not inject
        // an ExtensionResponse or approve any command other than this gate.
        cx.update(|window, cx| {
            let dialog_focus = app.read(cx).dialog_focus.clone();
            dialog_focus.focus(window, cx);
            window.draw(cx).clear(cx);
        });
        dispatch_named_key(cx, &approval_key);
    }
    Err(format!(
        "timed out after {} seconds waiting for native tool gate {}",
        TURN_TIMEOUT.as_secs(),
        gate.file_name()
    ))
}

fn focus_composer(cx: &mut VisualTestContext, app: &gpui::Entity<FarcasterApp>) {
    cx.update(|window, cx| {
        let composer_focus = app.read(cx).composer_focus.clone();
        composer_focus.focus(window, cx);
        window.draw(cx).clear(cx);
    });
}

fn dispatch_keystroke(cx: &mut VisualTestContext, keystroke: Keystroke) {
    // `simulate_input` and `simulate_keystrokes` call GPUI's unbounded test
    // executor drain. A full FarcasterApp has permanent receiver tasks, so
    // dispatch through the same Window path without draining those tasks.
    cx.update(|window, cx| {
        window.dispatch_keystroke(keystroke, cx);
        window.draw(cx).clear(cx);
    });
}

fn type_text(cx: &mut VisualTestContext, text: &str) {
    for character in text.chars() {
        let key = character.to_string();
        dispatch_keystroke(
            cx,
            Keystroke {
                modifiers: Modifiers::default(),
                key: key.clone(),
                key_char: Some(key),
            },
        );
    }
}

fn dispatch_named_key(cx: &mut VisualTestContext, key: &str) {
    dispatch_keystroke(cx, Keystroke::parse(key).expect("valid test key"));
}

fn type_and_enter(cx: &mut VisualTestContext, text: &str) {
    type_text(cx, text);
    dispatch_named_key(cx, "enter");
}

fn type_and_queue(cx: &mut VisualTestContext, text: &str) {
    type_text(cx, text);
    // The actual composer binds Tab to SubmitFollowUp while a run is active.
    dispatch_named_key(cx, "tab");
}

fn distinct_escape(cx: &mut VisualTestContext) {
    dispatch_named_key(cx, "escape");
}

fn held_escape(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.dispatch_event(
            KeyDownEvent {
                keystroke: Keystroke::parse("escape").expect("valid Escape key"),
                is_held: true,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
    });
}

fn select_requested_model(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    model_id: &str,
) -> Result<(), String> {
    let model = cx
        .update(|_, cx| {
            app.read(cx)
                .snapshot
                .models
                .iter()
                .find(|model| model.id == model_id)
                .cloned()
        })
        .ok_or_else(|| {
            format!("FARCASTER_E2E_MODEL {model_id:?} was not in the selected harness catalog")
        })?;
    cx.update(|_, cx| app.update(cx, |app, cx| app.select_model(&model, cx)));
    wait_for(cx, app, "the selected draft model prefill", |app| {
        app.snapshot
            .prefill_model
            .as_ref()
            .is_some_and(|current| current.id == model_id)
    })
}

fn require_live_access_mode(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    requested: HarnessAccessMode,
) -> Result<(), String> {
    let (configuration_status, available_access_modes, current_access_mode) = cx.update(|_, cx| {
        let snapshot = &app.read(cx).snapshot;
        (
            snapshot.configuration_status.clone(),
            snapshot.available_access_modes(),
            snapshot.access_mode,
        )
    });
    if let ConfigurationStatus::Failed(error) = configuration_status {
        return Err(format!(
            "E2E_BLOCKED: selected harness configuration failed before the live UI prompt: {error}; current access mode={current_access_mode:?}; available access modes={available_access_modes:?}"
        ));
    }
    if !available_access_modes.contains(&requested) {
        return Err(format!(
            "E2E_BLOCKED: selected draft does not offer required access mode {requested:?}; current access mode={current_access_mode:?}; available access modes={available_access_modes:?}"
        ));
    }
    cx.update(|_, cx| app.update(cx, |app, cx| app.set_access_mode(requested, cx)));
    wait_for(cx, app, "the requested live access mode", |app| {
        app.snapshot.access_mode == requested
    })
}

fn load_draft_configuration(
    cx: &mut VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
) -> Result<(), String> {
    // This is the production path that starts catalog discovery. A staged
    // draft begins in `Loading`, but bootstrap deliberately does not launch a
    // configuration process until the runtime picker opens.
    phase("configuration-picker-open");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.set_runtime_picker_open(true, window, cx));
        window.draw(cx).clear(cx);
    });
    let loaded = wait_for_with_timeout(
        cx,
        app,
        "the selected draft configuration",
        CONFIGURATION_TIMEOUT,
        |app| app.snapshot.configuration_status != ConfigurationStatus::Loading,
    );
    // Restore the same app surface the real composer tests exercise, whether
    // the catalog succeeded or failed. This also prevents picker focus from
    // changing subsequent raw-key routing.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.set_runtime_picker_open(false, window, cx));
        window.draw(cx).clear(cx);
    });
    loaded.map_err(|error| {
        let (status, current, available) = cx.update(|_, cx| {
            let snapshot = &app.read(cx).snapshot;
            (
                snapshot.configuration_status.clone(),
                snapshot.access_mode,
                snapshot.available_access_modes(),
            )
        });
        format!(
            "E2E_BLOCKED: {error}; configuration={status:?}; current access mode={current:?}; available access modes={available:?}"
        )
    })
}

fn with_live_app(
    cx: &mut TestAppContext,
    exercise: impl FnOnce(
        &mut VisualTestContext,
        &gpui::Entity<FarcasterApp>,
        &Path,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let config = live_config()?;
    // The UI app speaks to the installed harness directly. Its test process is
    // not a registered Farcaster MCP caller, so do not let an inherited host
    // MCP configuration add an unrelated server failure to model context.
    let _mcp = live_e2e_support::McpGuard::disabled();
    phase("config-validated");
    let case_dir = live_e2e_support::e2e_case_dir()?;
    let project = tempfile::tempdir_in(&case_dir)
        .map_err(|error| format!("create isolated E2E project: {error}"))?;
    fs::write(
        project.path().join("AGENTS.md"),
        "# Farcaster live UI E2E fixture\n\nUse only files in this directory. Do not inspect or modify parent directories. Run only the exact shell command requested by the prompt.\n",
    )
    .map_err(|error| format!("write isolated E2E project instructions: {error}"))?;
    phase("project-created");
    let project_path = project
        .path()
        .canonicalize()
        .map_err(|error| format!("canonicalize isolated E2E project: {error}"))?;
    StateStore::open()
        .and_then(|store| store.save_preferred_harness(config.harness))
        .map_err(|error| format!("save selected live harness in isolated state: {error}"))?;
    phase("state-store-seeded");

    // FarcasterApp owns a foreground runtime-wake task. The real supervisor
    // thread wakes it after native I/O; GPUI's default deterministic scheduler
    // rejects that cross-thread wake at test teardown. This is GPUI's supported
    // mixed deterministic/real-I/O mode. The case still owns all timing through
    // bounded wall-clock waits and drains projection explicitly below.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        install_component_theme(cx);
        cx.bind_keys(super::ui::keybindings::bindings());
    });
    phase("gpui-initialized");
    let (workgraph_updates, workgraph_rx) = async_channel::unbounded();
    let (worker_updates, worker_rx) = async_channel::unbounded();
    phase("window-creating");
    // `add_window_view` unconditionally runs GPUI's scheduler to quiescence.
    // Bootstrap owns permanent runtime/update receiver tasks, so use the same
    // real window/app construction without that unbounded test-only drain.
    let window = cx.add_window(|window, cx| {
        FarcasterApp::new(
            project_path.clone(),
            true,
            workgraph_rx,
            worker_rx,
            window,
            cx,
        )
    });
    let app = window
        .root(cx)
        .map_err(|error| format!("read FarcasterApp window root: {error}"))?;
    let window: AnyWindowHandle = window.into();
    let cx = VisualTestContext::from_window(window, cx).into_mut();
    phase("window-created");
    // Keep actual subscription senders alive for the full app test. They are
    // deliberately empty; the runtime/session supplies every observed event.
    let _updates = (workgraph_updates, worker_updates);
    focus_composer(cx, &app);
    phase("composer-focused");
    // `NewSession` stages a disconnected draft; the first real composer
    // prompt owns session startup. Waiting for `connected` here deadlocks a
    // correct lazy-start app before it can send that prompt.
    wait_for(cx, &app, "the selected disconnected draft", |app| {
        app.snapshot.harness == Some(config.harness)
            && app.snapshot.project == project_path
            && !app.snapshot.connected
            && app.snapshot.session.is_none()
            && app.snapshot.selected_session.is_none()
    })?;
    // Access modes can depend on the actual selected harness/model catalog.
    // Do not assume a static adapter declaration covers this disconnected
    // draft: open the real runtime picker to start production catalog loading,
    // then fail safely if it cannot offer the explicitly required live policy.
    load_draft_configuration(cx, &app)?;
    // No prompt has been sent yet. Force the shared live-test default
    // (Sandboxed unless the caller explicitly set `...ACCESS_MODE=full`) before
    // the bounded shell gate can run.
    require_live_access_mode(cx, &app, config.access_mode)?;
    if let Some(model) = config.model.as_deref() {
        wait_for(cx, &app, "the selected harness catalog", |app| {
            !app.snapshot.models.is_empty()
        })?;
        select_requested_model(cx, &app, model)?;
        // A model can narrow the available modes. Revalidate then reassert
        // the requested sandbox policy while still disconnected, before the
        // first prompt starts any native harness process.
        require_live_access_mode(cx, &app, config.access_mode)?;
    }
    exercise(cx, &app, &project_path)
}

/// Proves first Escape reaches the actual root key route, interrupts an active
/// native tool turn, and promotes both app input modes before the gate opens.
#[gpui::test]
#[ignore = "runs the selected installed harness and real model through the Farcaster GPUI app"]
fn live_e2e_ui_first_escape_applies_steer_and_queue(cx: &mut TestAppContext) {
    with_live_app(cx, |cx, app, project| {
        let gate = new_turn_gate(project, "ui-first")?;
        let _release_gate = ReleaseGate(gate.clone());
        focus_composer(cx, app);
        phase("first:gate-typed");
        type_and_enter(cx, &gate.prompt());
        submission_diagnostics(cx, app, "first:after-gate-submit", &[gate.file_name()]);
        wait_for_gate_tool(cx, app, &gate)?;
        wait_for(cx, app, "the active gate turn before steers", |app| {
            app.snapshot.conversation.running
        })?;

        let steer = format!("FARCASTER_UI_STEER_{}", uuid::Uuid::new_v4().simple());
        let queued = format!("FARCASTER_UI_QUEUE_{}", uuid::Uuid::new_v4().simple());
        let steer_message =
            format!("Stop the gated command. Include this token in your reply: {steer}. If pending messages request other tokens, include all of them in the same reply. Do not use tools.");
        let queued_message =
            format!("Include this token in your reply: {queued}. If pending messages request other tokens, include all of them in the same reply. Do not use tools.");
        focus_composer(cx, app);
        phase("first:steer-typed");
        type_and_enter(cx, &steer_message);
        submission_diagnostics(cx, app, "first:after-steer-submit", &[&steer]);
        let (steer_submission_id, steer_raw_text) = cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(
                app.composer.read(cx).value().is_empty(),
                "Enter did not clear the composer for the steering submission"
            );
            let pending = exact_pending_submission(app, PromptMode::Steer, &steer_message)
                .expect("the steer lacks one exact unresolved submission");
            assert_eq!(pending.1.trim_end(), steer_message);
            let queue = visible_prompt_queue(app);
            assert_eq!(
                queue.steering,
                [pending.1.as_str()],
                "the production composer did not render the unresolved steer immediately"
            );
            assert!(queue.follow_up.is_empty());
            assert!(
                user_rows(app, &steer).is_empty(),
                "the unresolved steer reached the transcript before native delivery"
            );
            pending
        });
        focus_composer(cx, app);
        phase("first:queue-typed");
        type_and_queue(cx, &queued_message);
        submission_diagnostics(cx, app, "first:after-queue-submit", &[&steer, &queued]);
        let queued_submission_id = cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(
                app.composer.read(cx).value().is_empty(),
                "Tab did not clear the composer for the follow-up submission"
            );
            let queued_pending =
                exact_pending_submission(app, PromptMode::FollowUp, &queued_message)
                    .expect("the follow-up lacks one exact unresolved submission");
            assert_eq!(queued_pending.1.trim_end(), queued_message);
            let queue = visible_prompt_queue(app);
            assert_eq!(queue.steering, [steer_raw_text.as_str()]);
            assert_eq!(queue.follow_up, [queued_pending.1.as_str()]);
            assert!(
                user_rows(app, &steer).is_empty() && user_rows(app, &queued).is_empty(),
                "unresolved steer or follow-up reached the transcript before native delivery"
            );
            let steer_pending = exact_pending_submission(app, PromptMode::Steer, &steer_message)
                .expect("the first unresolved submission disappeared before Escape");
            assert_eq!(
                steer_pending,
                (steer_submission_id.clone(), steer_raw_text.clone()),
                "the first unresolved submission changed identity before Escape",
            );
            queued_pending.0
        });
        assert_ne!(
            steer_submission_id, queued_submission_id,
            "steer and follow-up reused a submission ID"
        );
        gate.assert_process_alive()?;
        let completed_runs_before_apply =
            cx.update(|_, cx| app.read(cx).snapshot.conversation.completed_runs.len());

        focus_composer(cx, app);
        phase("first:escape-dispatched");
        distinct_escape(cx);
        assert!(
            cx.update(|_, cx| app.read(cx).composer_escape_armed.is_some()),
            "first Escape did not arm Abort after ApplySteering"
        );
        // Keep the gate closed: an ordinary completion cannot explain these
        // native tool/assistant transitions.
        wait_for(
            cx,
            app,
            "first-Escape steer delivery and reaction before gate release",
            |app| assistant_contains(app, &steer) && user_rows(app, &steer).len() == 1,
        )?;
        wait_for(
            cx,
            app,
            "first-Escape queue delivery and reaction before gate release",
            |app| assistant_contains(app, &queued) && user_rows(app, &queued).len() == 1,
        )?;
        wait_for(
            cx,
            app,
            "the first-Escape runtime handoff boundary",
            |app| apply_handoff_reached_boundary(app, &gate, completed_runs_before_apply),
        )?;
        gate.assert_still_closed()?;
        wait_for(cx, app, "exact delivered input rows", |app| {
            user_rows(app, &steer).len() == 1 && user_rows(app, &queued).len() == 1
        })?;
        assert!(
            cx.update(|_, cx| app.read(cx).composer.read(cx).value().is_empty()),
            "composer retained delivered steering or queue text"
        );
        Ok(())
    })
    .expect("live first-Escape UI E2E");
}

/// Uses a real platform held-key event. It must neither send another control
/// command nor change the arm created by a prior distinct Escape.
#[gpui::test]
#[ignore = "runs the selected installed harness and real model through the Farcaster GPUI app"]
fn live_e2e_ui_held_escape_does_not_double_apply(cx: &mut TestAppContext) {
    with_live_app(cx, |cx, app, project| {
        let gate = new_turn_gate(project, "ui-held")?;
        let _release_gate = ReleaseGate(gate.clone());
        focus_composer(cx, app);
        phase("held:gate-typed");
        type_and_enter(cx, &gate.prompt());
        submission_diagnostics(cx, app, "held:after-gate-submit", &[gate.file_name()]);
        wait_for_gate_tool(cx, app, &gate)?;
        wait_for(cx, app, "the active gate turn before held Escape", |app| {
            app.snapshot.conversation.running
        })?;
        let steer = format!("FARCASTER_UI_HELD_{}", uuid::Uuid::new_v4().simple());
        let steer_prompt = format!("Reply with this exact token: {steer}");
        focus_composer(cx, app);
        phase("held:steer-typed");
        type_and_enter(cx, &steer_prompt);
        submission_diagnostics(cx, app, "held:after-steer-submit", &[&steer]);
        wait_for_pending_or_visible_steer(cx, app, &steer_prompt, &steer)?;
        assert!(
            cx.update(|_, cx| user_rows(app.read(cx), &steer).is_empty()),
            "admitted held-test steer reached the transcript before native delivery"
        );
        gate.assert_process_alive()?;
        let completed_runs_before_apply =
            cx.update(|_, cx| app.read(cx).snapshot.conversation.completed_runs.len());
        focus_composer(cx, app);
        phase("held:first-escape-dispatched");
        distinct_escape(cx);
        let armed = cx.update(|_, cx| app.read(cx).composer_escape_armed.clone());
        assert!(armed.is_some(), "first distinct Escape did not arm Abort");
        phase("held:repeat-escape-dispatched");
        held_escape(cx);
        assert_eq!(
            cx.update(|_, cx| app.read(cx).composer_escape_armed.clone()),
            armed,
            "held Escape changed the real app abort arm"
        );
        wait_for(
            cx,
            app,
            "one first-Escape reaction before gate release",
            |app| assistant_rows(app, &steer).len() == 1 && user_rows(app, &steer).len() == 1,
        )?;
        wait_for(cx, app, "the held-key runtime handoff boundary", |app| {
            apply_handoff_reached_boundary(app, &gate, completed_runs_before_apply)
        })?;
        gate.assert_still_closed()?;
        Ok(())
    })
    .expect("live held-Escape UI E2E");
}

/// Proves that a second distinct Escape cancels both pending modes while the
/// original native tool gate remains closed, then leaves the session reusable.
#[gpui::test]
#[ignore = "runs the selected installed harness and real model through the Farcaster GPUI app"]
fn live_e2e_ui_second_escape_aborts_steer_and_queue(cx: &mut TestAppContext) {
    with_live_app(cx, |cx, app, project| {
        let gate = new_turn_gate(project, "ui-abort")?;
        let _release_gate = ReleaseGate(gate.clone());
        focus_composer(cx, app);
        phase("abort:gate-typed");
        type_and_enter(cx, &gate.prompt());
        submission_diagnostics(cx, app, "abort:after-gate-submit", &[gate.file_name()]);
        wait_for_gate_tool(cx, app, &gate)?;
        wait_for(cx, app, "the active gate turn before abort inputs", |app| {
            app.snapshot.conversation.running
        })?;
        let steer = format!("FARCASTER_UI_ABORT_{}", uuid::Uuid::new_v4().simple());
        let queued = format!("FARCASTER_UI_ABORT_QUEUE_{}", uuid::Uuid::new_v4().simple());
        focus_composer(cx, app);
        phase("abort:steer-typed");
        type_and_enter(cx, &format!("If invoked, reply with this token: {steer}"));
        submission_diagnostics(cx, app, "abort:after-steer-submit", &[&steer]);
        assert!(
            cx.update(|_, cx| {
                let app = app.read(cx);
                app.composer.read(cx).value().is_empty()
            }),
            "Enter did not clear the composer for the abort-test steer"
        );
        // As above, the real queue must render before the next UI key can
        // submit a follow-up; `can_submit` does not prove delivery.
        wait_for_exact_queue(
            cx,
            app,
            "the abort steer queue before the Tab follow-up",
            Some(&steer),
            None,
            true,
        )?;
        focus_composer(cx, app);
        phase("abort:queue-typed");
        type_and_queue(
            cx,
            &format!("If invoked after the steer, reply with this token: {queued}"),
        );
        submission_diagnostics(cx, app, "abort:after-queue-submit", &[&steer, &queued]);
        assert!(
            cx.update(|_, cx| {
                let app = app.read(cx);
                app.composer.read(cx).value().is_empty()
            }),
            "Tab did not clear the composer for the abort-test follow-up"
        );
        wait_for_exact_queue(
            cx,
            app,
            "the exact abort steer and follow-up queues",
            Some(&steer),
            Some(&queued),
            false,
        )?;
        assert!(
            cx.update(|_, cx| {
                let app = app.read(cx);
                user_rows(app, &steer).is_empty() && user_rows(app, &queued).is_empty()
            }),
            "admitted abort inputs reached the transcript before native delivery"
        );
        // This is a real shell command started by the selected installed
        // harness. The second Escape contract must stop it, not merely hide
        // its late UI events.
        gate.assert_process_alive()?;
        focus_composer(cx, app);
        phase("abort:first-escape-dispatched");
        distinct_escape(cx);
        phase("abort:second-escape-dispatched");
        distinct_escape(cx);
        assert!(
            cx.update(|_, cx| app.read(cx).composer_escape_armed.is_none()),
            "second distinct Escape did not consume the real app abort arm"
        );
        // The gate stays closed until this assertion has observed the native
        // abort outcome. Releasing it first would let natural completion hide
        // a missing Abort control request.
        wait_for(cx, app, "second-Escape abort before gate release", |app| {
            !app.snapshot.conversation.running
        })?;
        gate.assert_still_closed()?;
        gate.assert_process_exited_after_abort()?;
        // A harness can have streamed output before the second Escape wins.
        // It must not add a new cancelled handoff after this confirmed stop.
        // A row that appeared before stop remains only when its matching model
        // effect proves native delivery; the app does not expose a receipt
        // marker for every harness in this UI snapshot.
        let (
            steer_output_at_stop,
            queued_output_at_stop,
            steer_receipts_at_stop,
            queued_receipts_at_stop,
        ) = cx.update(|_, cx| -> Result<_, String> {
            let app = app.read(cx);
            assert!(
                !queued_steer(app, &steer) && !queued_follow_up(app, &queued),
                "Abort left an undelivered steer or follow-up in the visible queue"
            );
            Ok((
                assistant_rows(app, &steer)
                    .into_iter()
                    .map(|item| item.complete_text())
                    .collect::<Vec<_>>(),
                assistant_rows(app, &queued)
                    .into_iter()
                    .map(|item| item.complete_text())
                    .collect::<Vec<_>>(),
                delivered_abort_receipts(app, &steer)?,
                delivered_abort_receipts(app, &queued)?,
            ))
        })?;

        let after_abort = format!("FARCASTER_UI_AFTER_ABORT_{}", uuid::Uuid::new_v4().simple());
        focus_composer(cx, app);
        phase("abort:post-abort-normal-typed");
        type_and_enter(cx, &format!("Reply with this exact token: {after_abort}"));
        submission_diagnostics(cx, app, "abort:after-post-abort-submit", &[&after_abort]);
        wait_for(cx, app, "the post-abort normal prompt", |app| {
            assistant_contains(app, &after_abort) && user_rows(app, &after_abort).len() == 1
        })?;
        assert_eq!(
            cx.update(|_, cx| {
                assistant_rows(app.read(cx), &steer)
                    .into_iter()
                    .map(|item| item.complete_text())
                    .collect::<Vec<_>>()
            }),
            steer_output_at_stop,
            "a cancelled steer produced new output after the confirmed stop"
        );
        assert_eq!(
            cx.update(|_, cx| {
                assistant_rows(app.read(cx), &queued)
                    .into_iter()
                    .map(|item| item.complete_text())
                    .collect::<Vec<_>>()
            }),
            queued_output_at_stop,
            "a cancelled queue follow-up produced new output after the confirmed stop"
        );
        assert_eq!(
            cx.update(|_, cx| delivered_abort_receipts(app.read(cx), &steer)),
            Ok(steer_receipts_at_stop),
            "a cancelled steer gained or lost a transcript receipt after the confirmed stop"
        );
        assert_eq!(
            cx.update(|_, cx| delivered_abort_receipts(app.read(cx), &queued)),
            Ok(queued_receipts_at_stop),
            "a cancelled queue follow-up gained or lost a transcript receipt after the confirmed stop"
        );
        assert!(
            cx.update(|_, cx| app.read(cx).composer.read(cx).value().is_empty()),
            "composer retained text after the post-abort normal delivery"
        );
        Ok(())
    })
    .expect("live second-Escape UI E2E");
}
