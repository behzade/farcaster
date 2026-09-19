//! Opt-in, live account checks for the production runtime path.
//!
//! These tests deliberately start `RuntimeHandle`, rather than reducing
//! `SessionEvent`s into a conversation directly. They therefore cover the
//! supervisor, `RuntimeOwner`, durable prompt receipts, history replacement,
//! and selected-session routing as the app uses them. They do not use fixture
//! transports or inject protocol responses.
//!
//! Run one harness at a time through `scripts/e2e.sh`; that script creates the
//! required sibling `data` and `evidence` directories.
//!
//! `scripts/e2e.sh` creates and validates fresh state and evidence directories
//! before `StateStore::open` or a runtime thread starts. It retains the
//! caller's harness credentials and configuration, which live accounts need.
//! The test may leave a native session behind where the harness does not offer
//! deletion; every retained session belongs to the temporary project.
//!
//! There is deliberately no generic live `DeliveryUnknown` or rejection test:
//! each would need a harness-specific wire fault after a write, or an unsafe
//! malformed request. Production controls cannot make either race both safe
//! and repeatable across installed harnesses. Adapter process tests own those
//! receipt states; these tests prove the real accepted/restart/navigation path.
// Live-test progress is consumed by the E2E runner.
#![allow(clippy::print_stderr)]
use crate::agents::Backend;

use std::{
    collections::HashMap,
    fs,
    io::Write as _,
    path::{Component, Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    agents::{
        AgentLaunchConfig, PromptOutcome,
        extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptImage, PromptMode},
    },
    app::{
        infrastructure::persistence::state_path,
        runtime::{RuntimeCommand, RuntimeEvent, RuntimeHandle, RuntimeSnapshot},
    },
    conversation::TranscriptKind,
    sessions::SessionTarget,
};

use crate::agents::live_e2e_support::{
    McpGuard, TEST_IMAGE, TURN_TIMEOUT, alternate_image, bounded_command_permission, e2e_case_dir,
    isolated_locator_root, live_access_mode_for_harness, selected_live_harnesses,
};

const EVENT_POLL: Duration = Duration::from_millis(20);

struct RuntimeTrace {
    project: PathBuf,
    safe_gate: Option<String>,
    snapshot: Option<Arc<RuntimeSnapshot>>,
    outcomes: HashMap<String, PromptOutcome>,
    outcome_sessions: HashMap<String, Option<PathBuf>>,
    accepted_counts: HashMap<String, u64>,
    snapshot_count: u64,
    events: Vec<String>,
    evidence: PathBuf,
}

impl RuntimeTrace {
    fn new(project: &Path, safe_gate_message: Option<&str>) -> Result<Self, String> {
        let evidence_dir = std::env::var_os("FARCASTER_E2E_ARTIFACT_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| "runtime trace lacks FARCASTER_E2E_ARTIFACT_DIR".to_owned())?;
        let evidence = evidence_dir.join(format!("runtime-{}.log", unique("events")));
        fs::write(&evidence, format!("project={}\n", project.display())).map_err(|error| {
            format!(
                "create runtime E2E evidence {}: {error}",
                evidence.display()
            )
        })?;
        Ok(Self {
            project: project.into(),
            safe_gate: safe_gate_message.map(|message| gate_name(message).to_owned()),
            snapshot: None,
            outcomes: HashMap::new(),
            outcome_sessions: HashMap::new(),
            accepted_counts: HashMap::new(),
            snapshot_count: 0,
            events: Vec::new(),
            evidence,
        })
    }

    fn observe(&mut self, runtime: &RuntimeHandle, event: RuntimeEvent) -> Result<(), String> {
        match event {
            RuntimeEvent::Snapshot {
                generation,
                snapshot,
            } => {
                self.snapshot_count += 1;
                self.record(format!(
                    "snapshot generation={generation} harness={} session={:?} running={} items={}",
                    snapshot.harness.map(Backend::as_str).unwrap_or(""),
                    snapshot.live_session,
                    snapshot.conversation.running,
                    snapshot.conversation.items.len(),
                ))?;
                self.snapshot = Some(snapshot);
            }
            RuntimeEvent::PromptResult {
                target,
                outcome,
                session,
                ..
            } => {
                self.record(format!(
                    "prompt result {target}={outcome:?} session={session:?}"
                ))?;
                self.outcome_sessions.insert(target.clone(), session);
                if outcome == PromptOutcome::Accepted {
                    *self.accepted_counts.entry(target.clone()).or_default() += 1;
                }
                self.outcomes.insert(target, outcome);
                if outcome != PromptOutcome::Accepted {
                    return Err(format!(
                        "live prompt did not receive acceptance: {outcome:?}; {}",
                        self.summary()
                    ));
                }
            }
            RuntimeEvent::ExtensionUi { request, .. } => {
                self.record(format!("extension request {request:?}"))?;
                if let Some(response) =
                    approve_safe(&self.project, self.safe_gate.as_deref(), request)?
                {
                    runtime.send(RuntimeCommand::ExtensionResponse(response))?;
                }
            }
            RuntimeEvent::SystemNotification { title, body, .. } => {
                self.record(format!("notification {title}: {body}"))?;
            }
            RuntimeEvent::TurnCompletedNotification { body, .. } => {
                self.record(format!("notification Farcaster: Turn completed: {body}"))?;
            }
            RuntimeEvent::Stopped => return Err("production runtime stopped unexpectedly".into()),
            RuntimeEvent::SessionReset { .. }
            | RuntimeEvent::HistoryReset { .. }
            | RuntimeEvent::SessionTarget(_)
            | RuntimeEvent::Sessions { .. }
            | RuntimeEvent::SessionsFailed { .. }
            | RuntimeEvent::SessionMoved { .. }
            | RuntimeEvent::SessionDeleted { .. }
            | RuntimeEvent::RefreshCatalog
            | RuntimeEvent::SessionMetadata(_)
            | RuntimeEvent::SessionUpdated(_)
            | RuntimeEvent::AgentActivityUpdated(_)
            | RuntimeEvent::ExtensionUiDismissed { .. }
            | RuntimeEvent::SessionStatus { .. }
            | RuntimeEvent::ImportPreview { .. }
            | RuntimeEvent::ImportPreviewFailed { .. } => {}
        }
        Ok(())
    }

    fn summary(&self) -> String {
        self.events.join(" | ")
    }

    fn phase(&mut self, phase: &str) -> Result<(), String> {
        self.record(format!("phase {phase}"))
    }

    fn accepted_count(&self, target: &str) -> u64 {
        self.accepted_counts
            .get(target)
            .copied()
            .unwrap_or_default()
    }

    fn record(&mut self, event: String) -> Result<(), String> {
        self.events.push(event.clone());
        let mut evidence = fs::OpenOptions::new()
            .append(true)
            .open(&self.evidence)
            .map_err(|error| {
                format!(
                    "open runtime E2E evidence {}: {error}",
                    self.evidence.display()
                )
            })?;
        writeln!(evidence, "{event}").map_err(|error| {
            format!(
                "write runtime E2E evidence {}: {error}",
                self.evidence.display()
            )
        })
    }
}

#[derive(Default)]
struct LiveSessionCleanup {
    targets: Vec<SessionTarget>,
}

impl LiveSessionCleanup {
    fn track(&mut self, target: &SessionTarget) {
        if !self
            .targets
            .iter()
            .any(|known| known.path == target.path && known.harness == target.harness)
        {
            self.targets.push(target.clone());
        }
    }
}

impl Drop for LiveSessionCleanup {
    fn drop(&mut self) {
        for target in &self.targets {
            cleanup_created_session(target);
        }
    }
}

struct GateCleanup {
    project: PathBuf,
    message: String,
}

impl GateCleanup {
    fn new(project: &Path, message: &str) -> Self {
        Self {
            project: project.into(),
            message: message.into(),
        }
    }
}

impl Drop for GateCleanup {
    fn drop(&mut self) {
        if let Err(error) = release_gate(&self.project, &self.message) {
            eprintln!("live E2E gate cleanup failed: {error}");
        }
    }
}

#[test]
#[ignore = "uses one selected installed harness and a real model; requires FARCASTER_E2E_HARNESS"]
fn live_e2e_runtime_accepted_prompt_survives_restart_without_duplicate_or_replay()
-> Result<(), String> {
    isolated_live_runtime(
        "accepted_prompt_survives_restart_without_duplicate_or_replay",
        || {
            let _mcp = McpGuard::disabled();
            let harness = selected_harness()?;
            let project = live_project("restart")?;
            let config = live_config(harness)?;
            let draft_id = unique("restart-draft");
            let target = draft_target(&draft_id);
            let message = held_message("restart");
            let first_image = png_image();

            let mut sessions = LiveSessionCleanup::default();
            let runtime = start_runtime(harness, &project, &draft_id, config.clone())?;
            let _gate_cleanup = GateCleanup::new(&project, &message);
            let mut trace = RuntimeTrace::new(&project, Some(&message))?;
            trace.phase("submit initial draft prompt")?;
            runtime.send(prompt(target.clone(), message.clone(), first_image.clone()))?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.outcomes.get(&target) == Some(&PromptOutcome::Accepted)
                    && trace.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.harness == Some(harness)
                            && snapshot.conversation.running
                            && contains_tool_gate(snapshot, &message)
                            && snapshot.session_target().is_some()
                    })
            })?;
            let first = trace
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_target())
                .ok_or_else(|| {
                    format!(
                        "accepted live prompt has no session target: {}",
                        trace.summary()
                    )
                })?;
            sessions.track(&first);
            wait_for_gate_started(&runtime, &mut trace, &project, &message, TURN_TIMEOUT)?;
            require_user_rows(&trace, &message, std::slice::from_ref(&first_image))?;

            // This is an application restart, not an injected process event. Drop joins the
            // supervisor and closes its real adapter transport before the next runtime opens.
            trace.phase("drop runtime after witnessed initial shell")?;
            drop(runtime);

            let mut after_restart = RuntimeTrace::new(&project, Some(&message))?;
            let resumed = start_runtime(harness, &project, &draft_id, config)?;
            let restart_case = (|| -> Result<(), String> {
                after_restart.phase("restart persisted native session")?;
                resumed.send(RuntimeCommand::RestartSession {
                    path: first.path.clone(),
                    harness: first.harness,
                    session_id: first.id.clone(),
                    project: project.clone(),
                })?;
                wait_for(&resumed, &mut after_restart, TURN_TIMEOUT, |trace| {
                    trace.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.harness == Some(harness)
                            && snapshot.live_session.as_deref() == Some(first.path.as_path())
                            && snapshot.session_target().as_ref() == Some(&first)
                    })
                })?;
                after_restart.phase("wait for restarted production history receipt projection")?;
                wait_for(&resumed, &mut after_restart, TURN_TIMEOUT, |trace| {
                    require_user_rows(trace, &message, std::slice::from_ref(&first_image)).is_ok()
                })?;

                // A harness may resume the interrupted tool before its close becomes visible. Abort
                // only that real resumed run, then prove the original input was neither replayed nor
                // duplicated by Farcaster's accepted-receipt restoration.
                if after_restart
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.conversation.running)
                {
                    after_restart.phase("abort resumed active turn")?;
                    resumed.send(RuntimeCommand::Abort)?;
                    wait_for(&resumed, &mut after_restart, TURN_TIMEOUT, |trace| {
                        trace
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| !snapshot.conversation.running)
                    })?;
                }
                require_user_rows(&after_restart, &message, std::slice::from_ref(&first_image))?;

                // Equal native text with a different image proves receipt identity, not text, governs
                // transcript reconciliation after the restart.
                let later_target = bound_target(&first);
                let later_image = alternate_image();
                // The first gate only ends after the verified Abort. Its open file lets this later,
                // equal-text prompt settle instead of making a natural gate timeout look like a
                // replay result.
                release_gate(&project, &message)?;
                let snapshots_before_later = after_restart.snapshot_count;
                let accepted_before_later = after_restart.accepted_count(&later_target);
                after_restart.phase("submit equal-text image-distinct bound-session prompt")?;
                resumed.send(prompt(
                    later_target.clone(),
                    message.clone(),
                    later_image.clone(),
                ))?;
                wait_for(&resumed, &mut after_restart, TURN_TIMEOUT, |trace| {
                    trace.accepted_count(&later_target) > accepted_before_later
                        && trace.snapshot.as_ref().is_some_and(|snapshot| {
                            trace.snapshot_count > snapshots_before_later
                                && snapshot.conversation.running
                        })
                })?;
                let later_running_snapshot = after_restart.snapshot_count;
                wait_for(&resumed, &mut after_restart, TURN_TIMEOUT, |trace| {
                    trace.snapshot.as_ref().is_some_and(|snapshot| {
                        trace.snapshot_count > later_running_snapshot
                            && !snapshot.conversation.running
                            && snapshot.conversation.settled
                            && contains_assistant_token(snapshot, gate_name(&message))
                    })
                })?;
                require_user_rows(&after_restart, &message, &[first_image, later_image])?;
                Ok(())
            })();

            // Keep the real held shell bounded even when resumption or a later prompt fails.
            // Release it before closing the replacement runtime, then clean every known target.
            drop(_gate_cleanup);
            drop(resumed);
            restart_case
        },
    )
}

#[test]
#[ignore = "uses one selected installed harness and a real model; requires FARCASTER_E2E_HARNESS"]
fn live_e2e_runtime_navigation_keeps_pending_receipts_in_their_origin_session() -> Result<(), String>
{
    isolated_live_runtime(
        "navigation_keeps_pending_receipts_in_their_origin_session",
        || {
            let _mcp = McpGuard::disabled();
            let harness = selected_harness()?;
            let project = live_project("navigation")?;
            let config = live_config(harness)?;
            let first_draft = unique("navigation-a");
            let first_target = draft_target(&first_draft);
            let blocking_message = held_message("navigation-base");
            let queued_message = format!(
                "Reply with exactly {} and do not call a tool.",
                unique("navigation-queued")
            );
            let blocking_image = png_image();
            let queued_image = png_image();
            let second_image = alternate_image();
            let mut sessions = LiveSessionCleanup::default();
            let runtime = start_runtime(harness, &project, &first_draft, config)?;
            let _gate_cleanup = GateCleanup::new(&project, &blocking_message);
            let mut trace = RuntimeTrace::new(&project, Some(&blocking_message))?;
            trace.phase("submit first draft prompt and wait for shell witness")?;
            runtime.send(prompt(
                first_target.clone(),
                blocking_message.clone(),
                blocking_image,
            ))?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.outcomes.get(&first_target) == Some(&PromptOutcome::Accepted)
                    && trace.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.conversation.running
                            && contains_tool_gate(snapshot, &blocking_message)
                            && snapshot.session_target().is_some()
                    })
            })?;
            let first = trace
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_target())
                .ok_or_else(|| format!("first live session has no target: {}", trace.summary()))?;
            sessions.track(&first);
            wait_for_gate_started(
                &runtime,
                &mut trace,
                &project,
                &blocking_message,
                TURN_TIMEOUT,
            )?;

            // While the real first turn is held in its tool, send a true FollowUp to A. It is
            // not yet delivered. Navigate immediately, before waiting for any native receipt.
            // This is the production queued-before-navigation case, rather than a normal prompt
            // which may have already been delivered before selection changes.
            let queued_target = bound_target(&first);
            trace.phase("submit follow-up to first bound session before acknowledgement")?;
            runtime.send(prompt_with_mode(
                queued_target.clone(),
                PromptMode::FollowUp,
                queued_message.clone(),
                queued_image.clone(),
            ))?;

            // Navigate to B before releasing A's tool. B sends the same text but a different
            // attachment. The eventual A delivery must not bind to this active B transcript.
            let second_draft = unique("navigation-b");
            let second_target = draft_target(&second_draft);
            trace.phase("navigate to second draft before releasing first gate")?;
            runtime.send(RuntimeCommand::NewSession {
                id: second_draft,
                harness: harness.into(),
                project: project.clone(),
            })?;
            trace.phase("submit same-text image-distinct second draft prompt")?;
            runtime.send(prompt(
                second_target.clone(),
                queued_message.clone(),
                second_image.clone(),
            ))?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.outcomes.get(&second_target) == Some(&PromptOutcome::Accepted)
                    && trace.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.harness == Some(harness) && snapshot.session_target().is_some()
                    })
            })?;
            let second = trace
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_target())
                .ok_or_else(|| format!("second live session has no target: {}", trace.summary()))?;
            if first.path == second.path || first.id == second.id {
                return Err("navigation created no distinct second live session".into());
            }
            sessions.track(&second);
            require_outcome_session(&trace, &second_target, &second)?;
            require_user_rows(&trace, &queued_message, std::slice::from_ref(&second_image))?;

            // Release A only after B is selected. A's queued prompt now becomes eligible. Wait
            // for B to settle without changing its selected target, then assert its identical
            // text still has exactly its one local image row.
            trace.phase("release first shell gate while second session remains selected")?;
            release_gate(&project, &blocking_message)?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.session_target().as_ref() == Some(&second)
                        && snapshot.conversation.settled
                })
            })?;
            require_user_rows(&trace, &queued_message, std::slice::from_ref(&second_image))?;

            trace.phase("select first session to observe queued follow-up delivery")?;
            runtime.send(RuntimeCommand::SelectSession {
                path: first.path.clone(),
                harness: first.harness,
                session_id: first.id.clone(),
                project: project.clone(),
            })?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.selected_session.as_deref() == Some(first.path.as_path())
                        && snapshot.session_target().as_ref() == Some(&first)
                        && !snapshot.status.contains("Loading history")
                })
            })?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.session_target().as_ref() == Some(&first)
                        && snapshot.conversation.settled
                }) && require_user_rows(trace, &queued_message, std::slice::from_ref(&queued_image))
                    .is_ok()
            })?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.outcomes.get(&queued_target) == Some(&PromptOutcome::Accepted)
                    && trace
                        .outcome_sessions
                        .get(&queued_target)
                        .and_then(Option::as_deref)
                        == Some(first.path.as_path())
            })?;
            require_outcome_session(&trace, &queued_target, &first)?;
            require_user_rows(&trace, &queued_message, std::slice::from_ref(&queued_image))?;
            Ok(())
        },
    )
}

#[test]
#[ignore = "uses one selected installed harness and a real model; requires FARCASTER_E2E_HARNESS"]
/// This starts below `FarcasterApp`, so it proves runtime transport, native
/// delivery, and model effects. The GPUI live test owns immediate composer
/// queue presentation before acknowledgement.
fn live_e2e_runtime_accepted_steer_and_follow_up_queue_until_delivery() -> Result<(), String> {
    isolated_live_runtime("accepted_steer_and_follow_up_queue_until_delivery", || {
        let _mcp = McpGuard::disabled();
        let harness = selected_harness()?;
        let project = live_project("pre-delivery-receipts")?;
        let draft_id = unique("pre-delivery-draft");
        let initial_target = draft_target(&draft_id);
        let gate_message = held_message("pre-delivery");
        let steer_effect = unique("steer-delivered");
        let follow_up_effect = unique("follow-up-delivered");
        let steer_message = format!(
            "After the held shell task ends, include {steer_effect} exactly once in your reply. If other receipt instructions arrive with this one, include their tokens too. Do not use a tool."
        );
        let follow_up_message = format!(
            "After the held shell task ends, include {follow_up_effect} exactly once in your reply. If other receipt instructions arrive with this one, include their tokens too. Do not use a tool."
        );
        let steer_image = png_image();
        let follow_up_image = alternate_image();
        let mut trace = RuntimeTrace::new(&project, Some(&gate_message))?;
        let mut sessions = LiveSessionCleanup::default();
        let runtime = start_runtime(harness, &project, &draft_id, live_config(harness)?)?;
        let gate_cleanup = GateCleanup::new(&project, &gate_message);

        // Keep the actual shell gate bounded even when an assertion fails. The runtime
        // drops after this block, and the native session is then removed if the harness
        // exposes safe cleanup.
        let case = (|| -> Result<(), String> {
            trace.phase("submit held initial prompt")?;
            runtime.send(prompt(
                initial_target.clone(),
                gate_message.clone(),
                png_image(),
            ))?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.outcomes.get(&initial_target) == Some(&PromptOutcome::Accepted)
                    && trace.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.harness == Some(harness)
                            && snapshot.conversation.running
                            && contains_tool_gate(snapshot, &gate_message)
                            && snapshot.session_target().is_some()
                    })
            })?;
            let target = trace
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_target())
                .ok_or_else(|| {
                    format!(
                        "held live prompt has no session target: {}",
                        trace.summary()
                    )
                })?;
            sessions.track(&target);
            wait_for_gate_started(&runtime, &mut trace, &project, &gate_message, TURN_TIMEOUT)?;

            // Send both runtime commands while the real turn remains held. This
            // layer has no local composer queue; native queue snapshots can only
            // appear after the adapter responds, so UI timing is tested above it.
            let target = bound_target(&target);
            trace.phase("submit steer while actual shell remains held")?;
            runtime.send(prompt_with_mode(
                target.clone(),
                PromptMode::Steer,
                steer_message.clone(),
                steer_image.clone(),
            ))?;
            trace.phase("submit follow-up before waiting for the steer response")?;
            runtime.send(prompt_with_mode(
                target.clone(),
                PromptMode::FollowUp,
                follow_up_message.clone(),
                follow_up_image.clone(),
            ))?;
            trace.phase("check the last observed snapshot before native delivery")?;
            let snapshot = trace.snapshot.as_ref().ok_or_else(|| {
                format!(
                    "runtime omitted the pre-delivery snapshot: {}",
                    trace.summary()
                )
            })?;
            let steer_rows = snapshot
                .conversation
                .items
                .iter()
                .filter(|item| {
                    item.kind == TranscriptKind::User && item.complete_text() == steer_message
                })
                .count();
            let follow_up_rows = snapshot
                .conversation
                .items
                .iter()
                .filter(|item| {
                    item.kind == TranscriptKind::User && item.complete_text() == follow_up_message
                })
                .count();
            trace.record(format!(
                "pre-delivery queues steering={} follow_up={} user_rows steer={} follow_up={}",
                snapshot.conversation.queue.steering.len(),
                snapshot.conversation.queue.follow_up.len(),
                steer_rows,
                follow_up_rows,
            ))?;
            if steer_rows != 0 || follow_up_rows != 0 {
                return Err(format!(
                    "acknowledgement projected queued input into the user transcript before native delivery: steer_rows={steer_rows}, follow_up_rows={follow_up_rows}; {}",
                    trace.summary()
                ));
            }
            if checked_gate_path(&project, &gate_message)?.exists() {
                return Err("live shell gate released before the pre-delivery assertion".into());
            }

            trace.phase("release held shell and wait for native delivery/model effects")?;
            release_gate(&project, &gate_message)?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.conversation.settled
                        && !snapshot.conversation.running
                        && contains_assistant_token(snapshot, &steer_effect)
                        && contains_assistant_token(snapshot, &follow_up_effect)
                        && !snapshot
                            .conversation
                            .queue
                            .steering
                            .iter()
                            .any(|queued| queued == &steer_message)
                        && !contains_follow_up_queue(snapshot, &follow_up_message)
                }) && require_user_rows(trace, &steer_message, std::slice::from_ref(&steer_image))
                    .is_ok()
                    && require_user_rows(
                        trace,
                        &follow_up_message,
                        std::slice::from_ref(&follow_up_image),
                    )
                    .is_ok()
            })?;
            trace.phase("require two successful outcomes after delivery projection")?;
            wait_for(&runtime, &mut trace, TURN_TIMEOUT, |trace| {
                trace.accepted_count(&target) == 2
            })?;
            let snapshot = trace.snapshot.as_ref().ok_or_else(|| {
                format!(
                    "runtime omitted the post-delivery snapshot: {}",
                    trace.summary()
                )
            })?;
            trace.record(format!(
                "post-delivery queues steering={} follow_up={} user_rows steer={} follow_up={}",
                snapshot.conversation.queue.steering.len(),
                snapshot.conversation.queue.follow_up.len(),
                snapshot
                    .conversation
                    .items
                    .iter()
                    .filter(|item| {
                        item.kind == TranscriptKind::User && item.complete_text() == steer_message
                    })
                    .count(),
                snapshot
                    .conversation
                    .items
                    .iter()
                    .filter(|item| {
                        item.kind == TranscriptKind::User
                            && item.complete_text() == follow_up_message
                    })
                    .count(),
            ))?;
            Ok(())
        })();

        drop(gate_cleanup);
        drop(runtime);
        case
    })
}

fn selected_harness() -> Result<Backend, String> {
    let selected = selected_live_harnesses()?;
    match selected.as_slice() {
        [harness] => harness.parse(),
        _ => Err(
            "live runtime E2E requires exactly one FARCASTER_E2E_HARNESS; run each harness separately"
                .into(),
        ),
    }
}

fn draft_target(draft_id: &str) -> String {
    format!("draft:{draft_id}")
}

fn bound_target(session: &SessionTarget) -> String {
    format!(
        "session:{}",
        crate::sessions::normalize_session_path(&session.path).display()
    )
}

fn isolated_live_runtime(
    name: &str,
    run: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    e2e_case_dir().map_err(|error| format!("{name}: {error}"))?;
    assert_isolated_state().map_err(|error| format!("{name}: {error}"))?;
    run().map_err(|error| format!("{name}: {error}"))
}

fn assert_isolated_state() -> Result<(), String> {
    let root = std::env::var_os("FARCASTER_DATA_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "isolated live runtime child lacks FARCASTER_DATA_DIR".to_owned())?;
    if !root.is_absolute() {
        return Err(format!(
            "isolated data root is not absolute: {}",
            root.display()
        ));
    }
    let evidence = std::env::var_os("FARCASTER_E2E_ARTIFACT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "isolated live runtime child lacks FARCASTER_E2E_ARTIFACT_DIR".to_owned())?;
    if root.file_name().and_then(|name| name.to_str()) != Some("data")
        || evidence.file_name().and_then(|name| name.to_str()) != Some("evidence")
        || root.parent() != evidence.parent()
        || !root.is_dir()
        || !evidence.is_dir()
    {
        return Err(format!(
            "live runtime state/evidence escaped the isolated case: data={}, evidence={}",
            root.display(),
            evidence.display()
        ));
    }
    let expected = root.join("state.sqlite3");
    if state_path()? != expected {
        return Err(format!(
            "runtime state escaped test root: expected {}, got {}",
            expected.display(),
            state_path()?.display()
        ));
    }
    crate::app::persistence::open()?;
    Ok(())
}

fn live_project(label: &str) -> Result<PathBuf, String> {
    let project = e2e_case_dir()?.join("projects").join(unique(label));
    fs::create_dir_all(&project).map_err(|error| {
        format!(
            "create isolated live project {}: {error}",
            project.display()
        )
    })?;
    let project = project
        .canonicalize()
        .map_err(|error| format!("canonicalize isolated live project: {error}"))?;
    fs::write(
        project.join("AGENTS.md"),
        "# Live E2E test project\n\nOnly inspect or run the exact project-local gate file named in the user request. Do not access files outside this project.\n",
    )
    .map_err(|error| format!("write isolated project instructions: {error}"))?;
    Ok(project)
}

fn live_config(harness: Backend) -> Result<AgentLaunchConfig, String> {
    Ok(AgentLaunchConfig {
        program: PathBuf::from(harness.as_str()),
        prefix_args: Vec::new(),
        // Sandboxed is the default. A caller must explicitly opt into Full
        // through FARCASTER_E2E_ACCESS_MODE; we never fall back to it.
        access_mode: live_access_mode_for_harness(harness)?,
        app_proxy: None,
        session_locator_root: Some(isolated_locator_root()?),
    })
}

fn start_runtime(
    harness: Backend,
    project: &Path,
    draft_id: &str,
    config: AgentLaunchConfig,
) -> Result<RuntimeHandle, String> {
    let runtime = RuntimeHandle::spawn_with(project.into(), draft_id.into(), None, config);
    runtime.send(RuntimeCommand::NewSession {
        id: draft_id.into(),
        harness: harness.into(),
        project: project.into(),
    })?;
    Ok(runtime)
}

fn prompt(target: String, message: String, image: PromptImage) -> RuntimeCommand {
    prompt_with_mode(target, PromptMode::Normal, message, image)
}

fn prompt_with_mode(
    target: String,
    mode: PromptMode,
    message: String,
    image: PromptImage,
) -> RuntimeCommand {
    RuntimeCommand::Prompt {
        submission_id: uuid::Uuid::new_v4().to_string(),
        target,
        mode,
        message,
        display_message: None,
        invocation: None,
        images: vec![image],
        allow_while_running: false,
    }
}

fn wait_for(
    runtime: &RuntimeHandle,
    trace: &mut RuntimeTrace,
    timeout: Duration,
    predicate: impl Fn(&RuntimeTrace) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut next_heartbeat = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match runtime.try_recv() {
            Ok(event) => trace.observe(runtime, event)?,
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(EVENT_POLL),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("live runtime event channel closed".into());
            }
        }
        if predicate(trace) {
            return Ok(());
        }
        if Instant::now() >= next_heartbeat {
            trace.record(format!(
                "waiting events={} snapshots={} last={}",
                trace.events.len(),
                trace.snapshot_count,
                trace.events.last().map(String::as_str).unwrap_or("none")
            ))?;
            next_heartbeat += Duration::from_secs(5);
        }
    }
    trace.record(format!("timeout after {} seconds", timeout.as_secs()))?;
    Err(format!(
        "timed out after {} seconds waiting for production runtime state: {}",
        timeout.as_secs(),
        trace.summary()
    ))
}

fn approve_safe(
    project: &Path,
    safe_gate: Option<&str>,
    request: ExtensionUiRequest,
) -> Result<Option<ExtensionUiResponse>, String> {
    match request {
        ExtensionUiRequest::Notify { .. }
        | ExtensionUiRequest::SetStatus { .. }
        | ExtensionUiRequest::SetWidget { .. }
        | ExtensionUiRequest::SetTitle { .. }
        | ExtensionUiRequest::SetEditorText { .. } => Ok(None),
        request => {
            let gate = safe_gate.ok_or_else(|| {
                "E2E_BLOCKED: refusing an interaction before a registered project-local gate"
                    .to_owned()
            })?;
            let _gate_path = checked_gate_name_path(project, gate)?;
            bounded_command_permission(&request, &[gate_command(gate)]).map(Some)
        }
    }
}

fn contains_tool_gate(snapshot: &RuntimeSnapshot, message: &str) -> bool {
    let gate = gate_name(message);
    snapshot.conversation.items.iter().any(|item| {
        item.kind == TranscriptKind::Tool
            && (item.complete_text().contains(gate) || item.tool_output.contains(gate))
    })
}

fn contains_follow_up_queue(snapshot: &RuntimeSnapshot, message: &str) -> bool {
    snapshot
        .conversation
        .queue
        .follow_up
        .iter()
        .any(|queued| queued == message)
}

fn contains_assistant_token(snapshot: &RuntimeSnapshot, token: &str) -> bool {
    snapshot
        .conversation
        .items
        .iter()
        .any(|item| item.kind == TranscriptKind::Assistant && item.complete_text().contains(token))
}

fn require_user_rows(
    trace: &RuntimeTrace,
    message: &str,
    expected_images: &[PromptImage],
) -> Result<(), String> {
    let snapshot = trace
        .snapshot
        .as_ref()
        .ok_or_else(|| format!("runtime never published a snapshot: {}", trace.summary()))?;
    let rows = snapshot
        .conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User && item.complete_text() == message)
        .collect::<Vec<_>>();
    if rows.len() != expected_images.len() {
        return Err(format!(
            "expected {} exact user rows after runtime receipt reconciliation, found {}; trace={}",
            expected_images.len(),
            rows.len(),
            trace.summary()
        ));
    }
    let mut expected = expected_images.to_vec();
    for row in rows {
        let Some(index) = expected
            .iter()
            .position(|image| row_matches_prompt_image(row, image))
        else {
            return Err(format!(
                "receipt row has the wrong image bytes or MIME for {message:?}: {}",
                image_summary(row)
            ));
        };
        expected.remove(index);
    }
    if expected.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "receipt rows omitted expected image MIME/bytes for {message:?}: {:?}",
            expected
                .iter()
                .map(|image| (&image.mime_type, image.data.len()))
                .collect::<Vec<_>>()
        ))
    }
}

fn row_matches_prompt_image(
    row: &crate::conversation::TranscriptItem,
    expected: &PromptImage,
) -> bool {
    let Some(expected) = crate::conversation::EncodedImage::from_prompt(expected) else {
        return false;
    };
    matches!(row.images.as_slice(), [image] if image.as_ref() == &expected)
}

fn image_summary(row: &crate::conversation::TranscriptItem) -> String {
    row.images
        .iter()
        .map(|image| format!("{}:{} bytes", image.mime_type(), image.bytes().len()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn png_image() -> PromptImage {
    PromptImage::new(TEST_IMAGE.into(), "image/png".into())
}

fn require_outcome_session(
    trace: &RuntimeTrace,
    target: &str,
    expected: &SessionTarget,
) -> Result<(), String> {
    if trace
        .outcome_sessions
        .get(target)
        .and_then(Option::as_deref)
        == Some(expected.path.as_path())
    {
        return Ok(());
    }
    Err(format!(
        "prompt result {target} belonged to {:?}, not the expected session {}; trace={}",
        trace.outcome_sessions.get(target),
        expected.path.display(),
        trace.summary()
    ))
}

fn held_message(label: &str) -> String {
    let gate = unique(&format!("{label}-gate"));
    let command = gate_command(&gate);
    format!(
        "Use the shell tool to run exactly `{command}`. Do not answer before it exits. Then reply with the exact token {gate}."
    )
}

fn gate_command(gate: &str) -> String {
    format!(
        "true '{gate}'; printf started > '{gate}.started'; for i in $(seq 1 6000); do [ -s '{gate}' ] && cat '{gate}' && exit 0; sleep 0.1; done; exit 124"
    )
}

fn gate_name(message: &str) -> &str {
    message
        .split('`')
        .nth(1)
        .and_then(|command| command.split('\'').nth(1))
        .unwrap_or_default()
}

fn release_gate(project: &Path, message: &str) -> Result<(), String> {
    let path = checked_gate_path(project, message)?;
    fs::write(&path, "release\n")
        .map_err(|error| format!("release live gate {}: {error}", path.display()))
}

fn wait_for_gate_started(
    runtime: &RuntimeHandle,
    trace: &mut RuntimeTrace,
    project: &Path,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    let gate = checked_gate_path(project, message)?;
    let started = gate.with_file_name(format!(
        "{}.started",
        gate.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    ));
    let deadline = Instant::now() + timeout;
    let mut next_heartbeat = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if fs::read_to_string(&started).ok().as_deref() == Some("started") {
            if let Err(error) = trace.record(format!(
                "observed shell-start witness {}",
                started.display()
            )) {
                return Err(release_gate_after_failed_wait(project, message, error));
            }
            return Ok(());
        }
        match runtime.try_recv() {
            Ok(event) => {
                if let Err(error) = trace.observe(runtime, event) {
                    return Err(release_gate_after_failed_wait(project, message, error));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(EVENT_POLL),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(release_gate_after_failed_wait(
                    project,
                    message,
                    "live runtime event channel closed while waiting for the shell-start witness"
                        .into(),
                ));
            }
        }
        if Instant::now() >= next_heartbeat {
            if let Err(error) = trace.record(format!(
                "waiting for shell-start witness {} events={} snapshots={}",
                started.display(),
                trace.events.len(),
                trace.snapshot_count,
            )) {
                return Err(release_gate_after_failed_wait(project, message, error));
            }
            next_heartbeat += Duration::from_secs(5);
        }
    }
    Err(release_gate_after_failed_wait(
        project,
        message,
        format!(
            "timed out after {} seconds waiting for the actual project-local shell gate to start: {}",
            timeout.as_secs(),
            started.display()
        ),
    ))
}

fn release_gate_after_failed_wait(project: &Path, message: &str, error: String) -> String {
    match release_gate(project, message) {
        Ok(()) => format!("{error}; released the project-local gate during cleanup"),
        Err(cleanup) => format!("{error}; failed to release the project-local gate: {cleanup}"),
    }
}

fn checked_gate_path(project: &Path, message: &str) -> Result<PathBuf, String> {
    checked_gate_name_path(project, gate_name(message))
}

fn checked_gate_name_path(project: &Path, name: &str) -> Result<PathBuf, String> {
    let gate = Path::new(name);
    let mut components = gate.components();
    if !project.is_absolute()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(format!(
            "live gate is not an isolated project-local file: {}",
            gate.display()
        ));
    }
    Ok(project.join(gate))
}

#[test]
fn runtime_gate_permission_allows_only_the_exact_registered_command() {
    let project = Path::new("/private/tmp/farcaster-live-runtime-permission");
    let gate = "farcaster-live-gate";
    let title = format!(
        "Allow Bash?\n{}",
        serde_json::json!({"command": gate_command(gate), "description": "runtime gate"})
    );
    assert_eq!(
        approve_safe(
            project,
            Some(gate),
            ExtensionUiRequest::Select {
                id: "permission-id".into(),
                title,
                options: vec!["Deny".into(), "Allow".into()],
                timeout: None,
            },
        ),
        Ok(Some(ExtensionUiResponse::Value {
            id: "permission-id".into(),
            value: "Allow".into(),
        }))
    );
}

#[test]
fn runtime_gate_permission_rejects_a_compound_command() {
    let project = Path::new("/private/tmp/farcaster-live-runtime-permission");
    let gate = "farcaster-live-gate";
    let title = format!(
        "Allow Bash?\n{}",
        serde_json::json!({"command": format!("{}; touch not-allowed", gate_command(gate))})
    );
    let error = approve_safe(
        project,
        Some(gate),
        ExtensionUiRequest::Select {
            id: "permission-id".into(),
            title,
            options: vec!["Deny".into(), "Allow".into()],
            timeout: None,
        },
    )
    .expect_err("compound command must not be approved");
    assert!(error.contains("E2E_BLOCKED"));
}

fn unique(prefix: &str) -> String {
    format!("farcaster-live-{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn cleanup_created_session(target: &SessionTarget) {
    if let Err(error) = crate::agents::delete_session_family(std::slice::from_ref(target)) {
        eprintln!(
            "{} live E2E session retained at {} because cleanup is unavailable: {error}",
            target.harness,
            target.path.display()
        );
    }
}
