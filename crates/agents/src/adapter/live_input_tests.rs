//! Live input-control coverage.
//!
//! These tests run a fresh installed harness and a real model through the
//! production `SessionTransport`. They use an observed shell-tool gate rather
//! than a delay, fixture process, or injected activity. Set
//! `FARCASTER_E2E_HARNESS` to one harness when running them so every feature
//! reports its harness-specific result.

// Live-test diagnostics are consumed by the E2E runner.
#![allow(clippy::print_stderr)]
use std::time::Duration;

use crate::conversation::TranscriptKind;
use crate::{
    SessionOperation, SessionResponseErrorKind, SessionResponsePayload,
    extensions::{PromptImage, PromptMode},
};

use super::live_tests::{
    TEST_IMAGE, TURN_TIMEOUT,
    support::{
        LiveSession, PromptObservation, Submission, TurnGate, alternate_image, for_each_selected,
        image, marker,
    },
};

const RECEIPT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
struct Input {
    submission: Submission,
    marker: String,
    effect: String,
    submitted_at: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AbortReceipt {
    Accepted,
    RejectedBeforeAcceptance,
}

struct UnrelatedProcess(std::process::Child);

impl UnrelatedProcess {
    fn start() -> Result<Self, String> {
        let child = std::process::Command::new("sleep")
            .arg("3600")
            .spawn()
            .map_err(|error| format!("start unrelated Abort sentinel: {error}"))?;
        eprintln!(
            "E2E_ABORT_SENTINEL: started owned fixture pid={}",
            child.id()
        );
        Ok(Self(child))
    }

    fn assert_alive(&mut self) -> Result<(), String> {
        if let Some(status) = self
            .0
            .try_wait()
            .map_err(|error| format!("check unrelated Abort sentinel: {error}"))?
        {
            return Err(format!("Abort stopped an unrelated fixture: {status}"));
        }
        eprintln!("E2E_ABORT_SENTINEL: alive pid={}", self.0.id());
        Ok(())
    }
}

impl Drop for UnrelatedProcess {
    fn drop(&mut self) {
        // Only the Child created above belongs to this cleanup.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_queue_runs_once_after_the_held_turn() -> Result<(), String> {
    run_input_case(|live| {
        live.require_available("follow-up", &live.capabilities().turns.follow_up)?;
        live.require_available("queue", &live.capabilities().turns.queue)?;
        live.require_functional_prompt_observation(PromptMode::FollowUp)?;
        let gate = live.start_gated_turn("queue-auto")?;
        let queued = submit_input(live, PromptMode::FollowUp, "queue-auto", Vec::new())?;
        // A native queued input may not receive its replay receipt until the
        // held turn reaches its boundary. Do not make queue execution depend
        // on an acknowledgement that has not become observable yet.
        let release = live.activity_cursor();
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        let base_response = wait_for_assistant_message_after(live, release, &gate.marker)?;
        if live.wait_for_functional_observation_after(
            release,
            &queued.submission,
            &queued.marker,
            TURN_TIMEOUT,
        )? == PromptObservation::CorrelatedDelivery
        {
            let delivered = delivered_position(live, &queued.submission.id)
                .ok_or_else(|| "queue delivery vanished from the live trace".to_owned())?;
            if delivered <= base_response {
                return Err(format!(
                    "queued input delivered before the held turn's real response; trace={}",
                    live.trace_summary()
                ));
            }
        }
        wait_for_assistant_message_after(live, base_response, &queued.effect)?;
        require_effect_and_exactly_once(live, &queued)?;
        require_accepted(live, &queued)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_cancel_one_duplicate_queue_row_preserves_tool_and_neighbor() -> Result<(), String>
{
    run_input_case(|live| {
        live.require_available("follow-up", &live.capabilities().turns.follow_up)?;
        let gate = live.start_gated_turn("cancel-row")?;
        let effect = marker("cancel-survivor");
        let text = format!("Reply with exactly {effect}. Do not use tools.");
        let cancelled = live.submit(
            PromptMode::FollowUp,
            &text,
            vec![image(TEST_IMAGE, "image/png")],
        )?;
        let survivor = live.submit(PromptMode::FollowUp, &text, vec![alternate_image()])?;
        live.cancel_prompt(&cancelled.id)?;
        let reply = live.wait_for_response(&cancelled.id, RECEIPT_TIMEOUT)?;
        if !reply
            .result
            .is_err_and(|error| error.kind == SessionResponseErrorKind::Cancelled)
        {
            return Err("row cancellation omitted exact cancelled response".into());
        }
        gate.assert_still_closed()?;
        gate.assert_process_alive()?;
        let release = live.activity_cursor();
        live.release_gate(&gate)?;
        live.wait_for_gate_tool_end_after(release, &gate, TURN_TIMEOUT)?;
        live.wait_for_functional_observation_after(release, &survivor, &text, TURN_TIMEOUT)?;
        live.wait_for_assistant_text(&effect, TURN_TIMEOUT)?;
        live.wait_for_native_idle(TURN_TIMEOUT)?;
        live.assert_no_delivery(&cancelled.id)?;
        live.assert_functional_submission_once(&survivor, &text)?;
        require_submission_accepted(live, &survivor)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_stale_cancel_does_not_reject_an_inflight_steer() -> Result<(), String> {
    run_input_case(|live| {
        if live.harness() != "codex-cli" {
            return Err("E2E_BLOCKED: this case exercises Codex native steer cancellation".into());
        }
        let start = live.activity_cursor();
        live.submit(
            PromptMode::Normal,
            "Do not use tools. Write a numbered list of 50 short distinct facts about the ocean.",
            Vec::new(),
        )?;
        live.wait_for_activity_after(start, TURN_TIMEOUT, |event| {
            event["type"] == "message_update"
        })?;
        let steer = submit_input(live, PromptMode::Steer, "stale-cancel", Vec::new())?;
        // Codex steers are already native-owned. A click from an old composer
        // snapshot must neither surface a cancel error nor claim cancellation.
        live.cancel_prompt(&steer.submission.id)?;
        live.cancel_prompt(&steer.submission.id)?;
        live.wait_for_functional_observation_after(
            steer.submitted_at,
            &steer.submission,
            &steer.marker,
            TURN_TIMEOUT,
        )?;
        live.wait_for_assistant_text(&steer.effect, TURN_TIMEOUT)?;
        live.wait_for_native_idle(TURN_TIMEOUT)?;
        live.cancel_prompt(&steer.submission.id)?;
        require_effect_and_exactly_once(live, &steer)?;
        require_accepted(live, &steer)?;
        if live.activities().iter().any(|event| {
            event.value["submissionId"] == steer.submission.id
                && event.value["status"] == "cancelled"
        }) {
            return Err("stale click fabricated cancellation of an in-flight steer".into());
        }
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_steer_applies_at_an_observed_turn_boundary() -> Result<(), String> {
    run_input_case(|live| {
        live.require_available("steer", &live.capabilities().turns.steer)?;
        live.require_functional_prompt_observation(PromptMode::Steer)?;
        let gate = live.start_gated_turn("steer-boundary")?;
        let steer = submit_input(live, PromptMode::Steer, "steer-boundary", Vec::new())?;
        let release = live.activity_cursor();
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        live.wait_for_gate_tool_end_after(release, &gate, TURN_TIMEOUT)?;
        let gate_boundary = gate_tool_end_position_after(live, release, &gate)?;
        live.wait_for_functional_observation_after(
            gate_boundary,
            &steer.submission,
            &steer.marker,
            TURN_TIMEOUT,
        )?;
        wait_for_assistant_message_after(live, gate_boundary, &steer.effect)?;
        require_effect_and_exactly_once(live, &steer)?;
        require_accepted(live, &steer)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_held_boundary_consumes_all_pending_steers_together() -> Result<(), String> {
    run_input_case(|live| {
        if !matches!(live.harness(), "pi" | "opencode") {
            return Err("E2E_BLOCKED: this batch test requires a Pi/OpenCode held boundary".into());
        }
        let gate = live.start_gated_turn("steer-batch")?;
        let tokens = [
            marker("batch-first"),
            marker("batch-second"),
            marker("batch-third"),
        ];
        let mut submissions = Vec::new();
        for token in &tokens {
            let text = format!(
                "Batch receipt token: {token}. In your next response, list ALL batch receipt tokens from the user messages you have received, in order. Do not use tools or repeat the earlier gate response."
            );
            let submission = live.submit(PromptMode::Steer, &text, Vec::new())?;
            submissions.push((submission, text));
        }
        gate.assert_still_closed()?;
        gate.assert_process_alive()?;
        let release = live.activity_cursor();
        live.release_gate(&gate)?;
        live.wait_for_activity_after(release, TURN_TIMEOUT, |event| {
            event["type"] == "message_end" && event["message"]["role"] == "assistant"
        })?;
        let first_reply = live
            .activities()
            .iter()
            .skip(release)
            .find(|event| {
                event.value["type"] == "message_end"
                    && event.value["message"]["role"] == "assistant"
            })
            .expect("observed assistant reply")
            .value
            .to_string();
        if !tokens.iter().all(|token| first_reply.contains(token)) {
            return Err(format!(
                "the first response after the held tool omitted a pending steer: {first_reply}"
            ));
        }
        live.wait_for_native_idle(TURN_TIMEOUT)?;
        for (submission, text) in &submissions {
            live.assert_functional_submission_once(submission, text)?;
            require_submission_accepted(live, submission)?;
        }
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_text_only_turn_consumes_all_pending_steers_together() -> Result<(), String> {
    run_input_case(|live| {
        live.require_available("steer", &live.capabilities().turns.steer)?;
        let start = live.activity_cursor();
        live.submit(PromptMode::Normal,
            "Do not use any tools. Write a numbered list of 100 short, distinct facts about the ocean. Finish the list in this response.", Vec::new())?;
        // Submit during observed model output, not after a timing delay and not
        // behind a tool gate (which would exercise the already-working path).
        live.wait_for_activity_after(start, TURN_TIMEOUT, |event| {
            event["type"] == "message_update"
        })?;
        let tokens = [
            marker("idle-batch-first"),
            marker("idle-batch-second"),
            marker("idle-batch-third"),
        ];
        let queued_at = live.activity_cursor();
        let mut submissions = Vec::new();
        for token in &tokens {
            let text = format!(
                "Batch receipt token: {token}. In your next response, list ALL batch receipt tokens from the user messages you have received, in order. Do not use tools."
            );
            submissions.push((live.submit(PromptMode::Steer, &text, Vec::new())?, text));
        }
        // Pi can admit the batch at turn_end before agent_settled; OpenCode
        // admits it after settlement. In either case inspect the very next
        // assistant reply, not a later response after all steers trickle in.
        live.wait_for_activity_after(queued_at, TURN_TIMEOUT, |event| {
            event["type"] == "message_end" && event["message"]["role"] == "assistant"
        })?;
        let after_base = live.activity_cursor();
        let reply = live.wait_for_activity_after(after_base, TURN_TIMEOUT, |event| {
            event["type"] == "message_end" && event["message"]["role"] == "assistant"
        })?;
        let text = reply.to_string();
        if !tokens.iter().all(|token| text.contains(token)) {
            return Err(format!(
                "first reply after text-only settlement omitted pending steers: {text}"
            ));
        }
        if live
            .activities()
            .iter()
            .skip(start)
            .any(|event| event.value["type"] == "tool_execution_start")
        {
            return Err("text-only batch regression unexpectedly used a tool boundary".into());
        }
        live.wait_for_native_idle(TURN_TIMEOUT)?;
        for (submission, text) in &submissions {
            live.assert_functional_submission_once(submission, text)?;
            require_submission_accepted(live, submission)?;
        }
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_escape_does_not_strand_an_undelivered_normal_prompt() -> Result<(), String> {
    run_input_case(|live| {
        if live.harness() != "opencode" {
            return Err(
                "E2E_BLOCKED: this regression targets OpenCode's pending normal input".into(),
            );
        }
        require_control_features(live)?;
        live.require_prompt_delivery_tracking(PromptMode::Normal)?;
        let gate = live.start_gated_turn("normal-before-escape")?;
        // Hold native execution to make the reported ordering deterministic:
        // Normal is admitted but not delivered when Escape arrives. In the
        // reported session recovered native steers occupied this execution;
        // the gate replaces that timing, not the native admission/delivery API.
        let normal = submit_input(live, PromptMode::Normal, "pending-normal", Vec::new())?;
        live.wait_for_activity_after(normal.submitted_at, RECEIPT_TIMEOUT, |event| {
            event["type"] == "prompt_delivery"
                && event["submissionId"] == normal.submission.id
                && event["status"] == "accepted"
        })?;
        live.assert_no_delivery(&normal.submission.id)?;
        let steer = submit_input(live, PromptMode::Steer, "normal-escape-steer", Vec::new())?;
        let handoff = live.activity_cursor();
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        live.wait_for_apply_handoff_after(handoff, &gate, TURN_TIMEOUT)?;
        live.wait_for_delivery_after(handoff, &steer.submission.id, TURN_TIMEOUT)?;
        wait_for_assistant_message_after(live, handoff, &steer.effect)?;
        live.wait_for_settled_after(handoff, TURN_TIMEOUT)?;
        gate.assert_still_closed()?;
        gate.assert_process_exited_after_abort()?;
        let state = live.load_state()?;
        let normal_deliveries = prompt_delivery_positions(live, &normal.submission.id, "delivered");
        eprintln!(
            "E2E_PENDING_NORMAL: normal={} steer={} streaming={} pending={} normal_deliveries={} steer_deliveries={}",
            normal.submission.id,
            steer.submission.id,
            state.is_streaming,
            state.pending_message_count,
            normal_deliveries.len(),
            prompt_delivery_positions(live, &steer.submission.id, "delivered").len(),
        );
        if normal_deliveries.len() != 1 {
            return Err(format!(
                "Escape delivered the steer and settled, but stranded the already-admitted normal prompt {}: expected 1 delivery, got {}",
                normal.submission.id,
                normal_deliveries.len(),
            ));
        }
        require_accepted(live, &normal)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_apply_steering_interrupts_and_handoffs_immediately() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        let gate = live.start_gated_turn("apply-immediate")?;
        let queued = submit_input(live, PromptMode::FollowUp, "apply-queue", Vec::new())?;
        let steer = submit_input(live, PromptMode::Steer, "apply-steer", Vec::new())?;

        let handoff = live.activity_cursor();
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        live.wait_for_apply_handoff_after(handoff, &gate, TURN_TIMEOUT)?;
        gate.assert_still_closed()?;
        live.wait_for_functional_observation_after(
            handoff,
            &queued.submission,
            &queued.marker,
            TURN_TIMEOUT,
        )?;
        live.wait_for_functional_observation_after(
            handoff,
            &steer.submission,
            &steer.marker,
            TURN_TIMEOUT,
        )?;
        wait_for_assistant_message_after(live, handoff, &queued.effect)?;
        wait_for_assistant_message_after(live, handoff, &steer.effect)?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        require_effect_and_exactly_once(live, &queued)?;
        require_effect_and_exactly_once(live, &steer)?;
        require_accepted(live, &queued)?;
        require_accepted(live, &steer)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_abort_stops_an_active_real_tool_turn() -> Result<(), String> {
    run_input_case(|live| {
        let mut sentinel = UnrelatedProcess::start()?;
        live.require_available("interrupt", &live.capabilities().turns.interrupt)?;
        let gate = live.start_gated_turn("abort-active")?;
        let aborted_at = live.activity_cursor();
        let abort = live.abort()?;
        require_control_success(live, &abort, SessionOperation::Abort)?;
        live.wait_for_settled_after(aborted_at, TURN_TIMEOUT)?;
        let settled_at = settled_position_after(live, aborted_at)?;
        gate.assert_process_exited_after_abort()?;
        sentinel.assert_alive()?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        prove_same_session_liveness(live)?;
        sentinel.assert_alive()?;
        live.assert_no_gate_tool_start_after(settled_at, &gate)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_abort_does_not_consume_queued_work() -> Result<(), String> {
    run_input_case(|live| {
        live.require_available("interrupt", &live.capabilities().turns.interrupt)?;
        live.require_available("follow-up", &live.capabilities().turns.follow_up)?;
        live.require_functional_prompt_observation(PromptMode::FollowUp)?;
        let gate = live.start_gated_turn("abort-queue")?;
        let first = submit_input(live, PromptMode::FollowUp, "abort-queue-first", Vec::new())?;
        let second = submit_input(live, PromptMode::FollowUp, "abort-queue-second", Vec::new())?;

        let aborted_at = live.activity_cursor();
        let abort = live.abort()?;
        require_control_success(live, &abort, SessionOperation::Abort)?;
        live.wait_for_settled_after(aborted_at, TURN_TIMEOUT)?;
        let settled_at = settled_position_after(live, aborted_at)?;
        gate.assert_process_exited_after_abort()?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        let liveness_at = live.activity_cursor();
        prove_same_session_liveness(live)?;
        live.wait_for_settled_after(liveness_at, TURN_TIMEOUT)?;
        require_abort_receipt(live, &first)?;
        require_abort_receipt(live, &second)?;
        // A receipt that won the race before `agent_settled` remains valid.
        // Cancellation forbids only a later delivery or model effect.
        assert_undelivered_never_started_after(live, &first.submission, settled_at)?;
        assert_undelivered_never_started_after(live, &second.submission, settled_at)?;
        assert_delivered_submissions_exactly_once(live, &[&first.submission, &second.submission])?;
        live.assert_no_gate_tool_start_after(settled_at, &gate)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_queue_and_steer_keep_separate_ownership() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        let gate = live.start_gated_turn("queue-steer")?;
        let queued = submit_input(live, PromptMode::FollowUp, "queue-steer-queue", Vec::new())?;
        let steer = submit_input(live, PromptMode::Steer, "queue-steer-steer", Vec::new())?;
        let release = live.activity_cursor();
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        live.wait_for_functional_observation_after(
            release,
            &steer.submission,
            &steer.marker,
            TURN_TIMEOUT,
        )?;
        live.wait_for_functional_observation_after(
            release,
            &queued.submission,
            &queued.marker,
            TURN_TIMEOUT,
        )?;
        require_effect_and_exactly_once(live, &steer)?;
        require_effect_and_exactly_once(live, &queued)?;
        require_accepted(live, &steer)?;
        require_accepted(live, &queued)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_queue_steer_and_apply_handoffs_all_inputs_once() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        let gate = live.start_gated_turn("queue-steer-apply")?;
        let queued = submit_input(
            live,
            PromptMode::FollowUp,
            "queue-steer-apply-queue",
            Vec::new(),
        )?;
        let steer = submit_input(
            live,
            PromptMode::Steer,
            "queue-steer-apply-steer",
            Vec::new(),
        )?;
        let handoff = live.activity_cursor();
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        live.wait_for_apply_handoff_after(handoff, &gate, TURN_TIMEOUT)?;
        gate.assert_still_closed()?;
        live.wait_for_functional_observation_after(
            handoff,
            &queued.submission,
            &queued.marker,
            TURN_TIMEOUT,
        )?;
        live.wait_for_functional_observation_after(
            handoff,
            &steer.submission,
            &steer.marker,
            TURN_TIMEOUT,
        )?;
        wait_for_assistant_message_after(live, handoff, &queued.effect)?;
        wait_for_assistant_message_after(live, handoff, &steer.effect)?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        require_effect_and_exactly_once(live, &queued)?;
        require_effect_and_exactly_once(live, &steer)?;
        require_accepted(live, &queued)?;
        require_accepted(live, &steer)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_queue_steer_and_abort_cancels_only_undelivered_handoff_work() -> Result<(), String>
{
    run_input_case(|live| {
        require_control_features(live)?;
        let gate = live.start_gated_turn("queue-steer-abort")?;
        let queued = submit_input(
            live,
            PromptMode::FollowUp,
            "queue-steer-abort-queue",
            Vec::new(),
        )?;
        let steer = submit_input(
            live,
            PromptMode::Steer,
            "queue-steer-abort-steer",
            Vec::new(),
        )?;
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        // Deliberately do not wait for either delivery: this is the second-Esc
        // race. Abort may find a local input or one the harness already owns.
        // The native cancellation result determines which claim this test can
        // make; a late receipt alone cannot establish that timing.
        let aborted_at = live.activity_cursor();
        let abort = live.abort()?;
        require_control_success(live, &abort, SessionOperation::Abort)?;
        live.wait_for_settled_after(aborted_at, TURN_TIMEOUT)?;
        let settled_at = settled_position_after(live, aborted_at)?;
        gate.assert_process_exited_after_abort()?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        let liveness_at = live.activity_cursor();
        prove_same_session_liveness(live)?;
        live.wait_for_settled_after(liveness_at, TURN_TIMEOUT)?;
        let queued_receipt = require_abort_receipt(live, &queued)?;
        let steer_receipt = require_abort_receipt(live, &steer)?;
        // Apply may already have committed a handoff when Abort reaches the
        // harness. A one-time, ID-correlated late delivery then belongs to the
        // original submission; it is not a replay. A proven local rejection
        // must never deliver. Anything else remains an explicit E2E limit.
        assert_mixed_handoff_abort_disposition(live, &queued, queued_receipt, settled_at)?;
        assert_mixed_handoff_abort_disposition(live, &steer, steer_receipt, settled_at)?;
        assert_delivered_submissions_exactly_once(live, &[&queued.submission, &steer.submission])?;
        live.assert_no_gate_tool_start_after(settled_at, &gate)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_receipt_races_keep_old_and_new_inputs_isolated() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        require_exact_delivery_tracking(live)?;
        let gate = live.start_gated_turn("receipt-race")?;
        let old = submit_input(
            live,
            PromptMode::FollowUp,
            "receipt-race-old",
            vec![image(TEST_IMAGE, "image/png")],
        )?;
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        let aborted_at = live.activity_cursor();
        let abort = live.abort()?;
        require_control_success(live, &abort, SessionOperation::Abort)?;
        live.wait_for_settled_after(aborted_at, TURN_TIMEOUT)?;
        let settled_at = settled_position_after(live, aborted_at)?;
        gate.assert_process_exited_after_abort()?;

        let newer = submit_input(
            live,
            PromptMode::Normal,
            "receipt-race-new",
            vec![alternate_image()],
        )?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        let old_receipt = require_abort_receipt(live, &old)?;
        if old_receipt == AbortReceipt::Accepted {
            require_late_old_delivery(live, &old, &newer)?;
        } else {
            live.assert_no_delivery(&old.submission.id)?;
            assert_no_transcript_user(live, &old.marker)?;
        }
        live.wait_for_delivery_after(newer.submitted_at, &newer.submission.id, TURN_TIMEOUT)?;
        require_effect_and_exactly_once(live, &newer)?;
        require_accepted(live, &newer)?;
        // A late event for old must stay tied to old; it cannot bind the newer
        // input merely because the two turns share one session.
        if old_receipt == AbortReceipt::Accepted {
            assert_retired_submission_isolated(live, &old, &newer)?;
        }
        prove_same_session_liveness(live)?;
        live.assert_no_gate_tool_start_after(settled_at, &gate)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_duplicate_text_and_images_keep_exact_submission_identity() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        require_exact_delivery_tracking(live)?;
        let gate = live.start_gated_turn("duplicate-images")?;
        let text = "Farcaster E2E duplicate text. Once this input is delivered, respond now without invoking any tools. Do not wait for, rerun, continue, or otherwise act on an earlier gate or tool request; leave any existing command untouched. Include the token FARCASTER_DUPLICATE_EFFECT exactly once in your final response.";
        let first = live.submit(
            PromptMode::FollowUp,
            text,
            vec![image(TEST_IMAGE, "image/png")],
        )?;
        let second = live.submit(PromptMode::Steer, text, vec![alternate_image()])?;
        let handoff = live.activity_cursor();
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        live.wait_for_apply_handoff_after(handoff, &gate, TURN_TIMEOUT)?;
        gate.assert_still_closed()?;
        live.wait_for_delivery_after(handoff, &first.id, TURN_TIMEOUT)?;
        live.wait_for_delivery_after(handoff, &second.id, TURN_TIMEOUT)?;
        wait_for_assistant_message_after(live, handoff, "FARCASTER_DUPLICATE_EFFECT")?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        live.wait_for_assistant_text("FARCASTER_DUPLICATE_EFFECT", TURN_TIMEOUT)?;
        live.assert_duplicate_submissions_once(&first, &second)?;
        require_submission_accepted(live, &first)?;
        require_submission_accepted(live, &second)?;
        prove_same_session_liveness(live)
    })
}

#[test]
#[ignore = "uses one installed harness and a real model; set FARCASTER_E2E_HARNESS"]
fn live_e2e_input_same_session_remains_live_after_every_control_path() -> Result<(), String> {
    run_input_case(|live| {
        require_control_features(live)?;
        let gate = live.start_gated_turn("liveness")?;
        let queued = submit_input(live, PromptMode::FollowUp, "liveness-queue", Vec::new())?;
        let apply = live.apply_steering()?;
        require_control_success(live, &apply, SessionOperation::ApplySteering)?;
        let abort_at = live.activity_cursor();
        let abort = live.abort()?;
        require_control_success(live, &abort, SessionOperation::Abort)?;
        live.wait_for_settled_after(abort_at, TURN_TIMEOUT)?;
        let settled_at = settled_position_after(live, abort_at)?;
        gate.assert_process_exited_after_abort()?;
        gate.assert_still_closed()?;
        live.release_gate(&gate)?;
        prove_same_session_liveness(live)?;
        require_abort_receipt(live, &queued)?;
        live.assert_no_gate_tool_start_after(settled_at, &gate)
    })
}

fn run_input_case(
    mut exercise: impl FnMut(&mut LiveSession) -> Result<(), String>,
) -> Result<(), String> {
    for_each_selected(|live| {
        // We require each selected harness to prove the advertised behavior.
        // Unsupported capability is an explicit failure, never a skipped pass.
        live.configure_steering()?;
        exercise(live)
    })
}

fn require_control_features(live: &LiveSession) -> Result<(), String> {
    live.require_available("interrupt", &live.capabilities().turns.interrupt)?;
    live.require_available("steer", &live.capabilities().turns.steer)?;
    live.require_available("follow-up", &live.capabilities().turns.follow_up)?;
    live.require_available("queue", &live.capabilities().turns.queue)?;
    live.require_functional_prompt_observation(PromptMode::Steer)?;
    live.require_functional_prompt_observation(PromptMode::FollowUp)?;
    Ok(())
}

fn require_exact_delivery_tracking(live: &LiveSession) -> Result<(), String> {
    live.require_prompt_delivery_tracking(PromptMode::Steer)?;
    live.require_prompt_delivery_tracking(PromptMode::FollowUp)
}

fn submit_input(
    live: &mut LiveSession,
    mode: PromptMode,
    label: &str,
    images: Vec<PromptImage>,
) -> Result<Input, String> {
    let input_marker = marker(label);
    let effect = marker("input-effect");
    let submitted_at = live.activity_cursor();
    let submission = live.submit(
        mode,
        format!(
            "Farcaster input {input_marker}. Once this input is delivered, respond now without invoking any tools. Do not wait for, rerun, continue, or otherwise act on an earlier gate or tool request; leave any existing command untouched. Include the token {effect} exactly once in your final response. If other Farcaster input instructions are delivered in this batch, include each of their effect tokens exactly once too. Do not echo {input_marker}."
        ),
        images,
    )?;
    Ok(Input {
        submission,
        marker: input_marker,
        effect,
        submitted_at,
    })
}

fn require_accepted(live: &mut LiveSession, input: &Input) -> Result<(), String> {
    require_submission_accepted(live, &input.submission)
}

fn require_submission_accepted(
    live: &mut LiveSession,
    submission: &Submission,
) -> Result<(), String> {
    let response = live.wait_for_response(&submission.id, RECEIPT_TIMEOUT)?;
    if response.operation() != SessionOperation::Prompt(submission.mode) {
        return Err(format!(
            "submission {} returned {:?}, expected prompt {:?}",
            submission.id,
            response.operation(),
            submission.mode
        ));
    }
    match response.result {
        Ok(SessionResponsePayload::Prompt(mode)) if mode == submission.mode => Ok(()),
        Ok(other) => Err(format!(
            "submission {} returned wrong prompt payload: {other:?}",
            submission.id
        )),
        Err(error) => Err(format!(
            "submission {} was not accepted by the live harness: {error}",
            submission.id
        )),
    }
}

/// A second Escape may find a prompt still local to the adapter, or may lose
/// the race to the harness receipt. Both are observable outcomes. A write with
/// no receipt is neither and must not become a passing cancellation claim.
fn require_abort_receipt(live: &mut LiveSession, input: &Input) -> Result<AbortReceipt, String> {
    let response = live.wait_for_response(&input.submission.id, RECEIPT_TIMEOUT)?;
    if response.operation() != SessionOperation::Prompt(input.submission.mode) {
        return Err(format!(
            "aborted submission {} returned {:?}, expected prompt {:?}",
            input.submission.id,
            response.operation(),
            input.submission.mode
        ));
    }
    match response.result {
        Ok(SessionResponsePayload::Prompt(mode)) if mode == input.submission.mode => {
            Ok(AbortReceipt::Accepted)
        }
        Ok(other) => Err(format!(
            "aborted submission {} returned wrong prompt payload: {other:?}",
            input.submission.id
        )),
        Err(error)
            if matches!(
                error.kind,
                SessionResponseErrorKind::RejectedBeforeAcceptance
                    | SessionResponseErrorKind::Cancelled
            ) =>
        {
            Ok(AbortReceipt::RejectedBeforeAcceptance)
        }
        Err(error) if error.kind == SessionResponseErrorKind::DeliveryUnknown => Err(format!(
            "E2E_BLOCKED: {} left aborted submission {} with unknown delivery: {error}",
            live.harness(),
            input.submission.id
        )),
        Err(error) => Err(format!(
            "E2E_BLOCKED: {} did not prove acceptance or pre-dispatch rejection for aborted submission {}: {error}",
            live.harness(),
            input.submission.id
        )),
    }
}

fn require_control_success(
    live: &mut LiveSession,
    id: &str,
    expected: SessionOperation,
) -> Result<(), String> {
    let response = live.wait_for_response(id, RECEIPT_TIMEOUT)?;
    if response.operation() != expected {
        return Err(format!(
            "control request {id} returned {:?}, expected {expected:?}",
            response.operation()
        ));
    }
    response
        .result
        .map(|_| ())
        .map_err(|error| format!("control request {id} failed: {error}"))
}

fn require_effect_and_exactly_once(live: &mut LiveSession, input: &Input) -> Result<(), String> {
    live.wait_for_assistant_text(&input.effect, TURN_TIMEOUT)?;
    live.assert_functional_submission_once(&input.submission, &input.marker)
}

fn prove_same_session_liveness(live: &mut LiveSession) -> Result<(), String> {
    live.require_functional_prompt_observation(PromptMode::Normal)?;
    // A final text delta can precede native turn settlement. A new Normal
    // prompt must wait for actual idle state, not race the old response tail.
    live.wait_for_native_idle(TURN_TIMEOUT)?;
    let later = submit_liveness_input(live)?;
    live.wait_for_functional_observation_after(
        later.submitted_at,
        &later.submission,
        &later.marker,
        TURN_TIMEOUT,
    )?;
    live.wait_for_assistant_text(&later.effect, TURN_TIMEOUT)?;
    // Streaming the token does not prove that the native client has finished
    // recording this turn. Wait for its real idle state before reading history.
    live.wait_for_native_idle(TURN_TIMEOUT)?;
    live.assert_functional_submission_once(&later.submission, &later.marker)?;
    require_accepted(live, &later)
}

fn submit_liveness_input(live: &mut LiveSession) -> Result<Input, String> {
    let input_marker = marker("same-session-liveness");
    let effect = marker("same-session-liveness-effect");
    let submitted_at = live.activity_cursor();
    let submission = live.submit(
        PromptMode::Normal,
        format!(
            "Farcaster liveness request {input_marker}. Handle only this new request and respond without invoking tools. Do not repeat, summarize, continue, execute, or act on any earlier user request, tool task, token, gate, or queued input; leave any earlier command untouched. Reply with exactly {effect}."
        ),
        Vec::new(),
    )?;
    Ok(Input {
        submission,
        marker: input_marker,
        effect,
        submitted_at,
    })
}

fn assert_no_delivery_after(
    live: &LiveSession,
    cursor: usize,
    submission_id: &str,
) -> Result<(), String> {
    live.assert_no_delivery_before(cursor, submission_id)
}

fn delivered_position(live: &LiveSession, submission_id: &str) -> Option<usize> {
    live.activities().iter().position(|event| {
        event.value["type"].as_str() == Some("prompt_delivery")
            && event.value["submissionId"].as_str() == Some(submission_id)
            && event.value["status"].as_str() == Some("delivered")
    })
}

fn wait_for_assistant_message_after(
    live: &mut LiveSession,
    cursor: usize,
    marker: &str,
) -> Result<usize, String> {
    live.wait_for_activity_after(cursor, TURN_TIMEOUT, |event| {
        event["type"].as_str() == Some("message_end")
            && event["message"]["role"].as_str() == Some("assistant")
            && event.to_string().contains(marker)
    })?;
    live.activities()
        .iter()
        .enumerate()
        .skip(cursor)
        .find_map(|(index, event)| {
            (event.value["type"].as_str() == Some("message_end")
                && event.value["message"]["role"].as_str() == Some("assistant")
                && event.value.to_string().contains(marker))
            .then_some(index)
        })
        .ok_or_else(|| format!("assistant completion {marker:?} vanished from trace"))
}

fn gate_tool_end_position_after(
    live: &LiveSession,
    cursor: usize,
    gate: &TurnGate,
) -> Result<usize, String> {
    let tool_call_id = live.gate_tool_call_id(gate)?;
    live.activities()
        .iter()
        .enumerate()
        .skip(cursor)
        .find_map(|(index, event)| {
            (event.value["type"].as_str() == Some("tool_execution_end")
                && event.value["toolCallId"].as_str() == Some(tool_call_id.as_str()))
            .then_some(index)
        })
        .ok_or_else(|| {
            format!(
                "exact gate tool end {tool_call_id} vanished from the activity trace; trace={}",
                live.trace_summary()
            )
        })
}

fn assert_undelivered_never_started_after(
    live: &mut LiveSession,
    submission: &Submission,
    settled_at: usize,
) -> Result<(), String> {
    if !live.tracks_prompt_delivery(submission.mode) {
        // Native history has no ID-correlated consumption event. It cannot
        // distinguish an allowed receipt from execution after Abort.
        eprintln!(
            "E2E_LIMIT: {} cannot prove exact post-settlement cancellation for untracked {:?} input {}",
            live.harness(),
            submission.mode,
            submission.id,
        );
        return Ok(());
    }
    if delivered_position(live, &submission.id).is_none_or(|position| position >= settled_at) {
        assert_no_delivery_after(live, settled_at, &submission.id)?;
    }
    Ok(())
}

/// Apply followed by Abort has a third ownership state that plain queued Abort
/// does not: the harness can have committed the handoff while cancellation is
/// in flight. A later correlated delivery must land exactly once. The common
/// event stream cannot prove whether a delivery after settlement predated the
/// native cancellation attempt, so that outcome remains limited.
fn assert_mixed_handoff_abort_disposition(
    live: &mut LiveSession,
    input: &Input,
    receipt: AbortReceipt,
    settled_at: usize,
) -> Result<(), String> {
    if !live.tracks_prompt_delivery(input.submission.mode) {
        eprintln!(
            "E2E_LIMIT: {} cannot resolve mixed Apply/Abort ownership for untracked {:?} input {}",
            live.harness(),
            input.submission.mode,
            input.submission.id,
        );
        return Ok(());
    }

    match receipt {
        AbortReceipt::RejectedBeforeAcceptance => {
            // This is the only outcome that proves the harness never owned
            // the input. A correlated delivery would contradict that result.
            live.assert_no_delivery(&input.submission.id)
        }
        AbortReceipt::Accepted => {
            let accepted = prompt_delivery_positions(live, &input.submission.id, "accepted");
            if accepted.len() != 1 {
                return Err(format!(
                    "accepted mixed-handoff submission {} emitted {} acceptance events; trace={}",
                    input.submission.id,
                    accepted.len(),
                    live.trace_summary()
                ));
            }

            let delivered = prompt_delivery_positions(live, &input.submission.id, "delivered");
            match delivered.len() {
                1 => {
                    live.assert_submission_once(&input.submission)?;
                    if delivered[0] >= settled_at {
                        eprintln!(
                            "E2E_LIMIT: {} delivered accepted mixed Apply/Abort submission {} after settlement; the common trace cannot prove whether native ownership predated Abort",
                            live.harness(),
                            input.submission.id,
                        );
                    }
                    Ok(())
                }
                0 => {
                    assert_no_transcript_user(live, &input.marker)?;
                    eprintln!(
                        "E2E_LIMIT: {} accepted mixed Apply/Abort submission {} but exposed no correlated delivery or rejection; native cancellation ownership remains unresolved",
                        live.harness(),
                        input.submission.id,
                    );
                    Ok(())
                }
                count => Err(format!(
                    "accepted mixed-handoff submission {} emitted {count} deliveries; expected one; trace={}",
                    input.submission.id,
                    live.trace_summary()
                )),
            }
        }
    }
}

fn assert_no_transcript_user(live: &LiveSession, marker: &str) -> Result<(), String> {
    if live
        .conversation()
        .items
        .iter()
        .any(|item| item.kind == TranscriptKind::User && item.complete_text().contains(marker))
    {
        Err(format!(
            "accepted-but-undelivered input {marker:?} created a user transcript row; transcript={}",
            live.transcript_summary()
        ))
    } else {
        Ok(())
    }
}

fn prompt_delivery_positions(live: &LiveSession, submission_id: &str, status: &str) -> Vec<usize> {
    live.activities()
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            (event.value["type"].as_str() == Some("prompt_delivery")
                && event.value["submissionId"].as_str() == Some(submission_id)
                && event.value["status"].as_str() == Some(status))
            .then_some(index)
        })
        .collect()
}

fn settled_position_after(live: &LiveSession, cursor: usize) -> Result<usize, String> {
    live.activities()
        .iter()
        .enumerate()
        .skip(cursor)
        .find_map(|(index, event)| {
            (event.value["type"].as_str() == Some("agent_settled")).then_some(index)
        })
        .ok_or_else(|| {
            format!(
                "confirmed cancellation settlement vanished from the activity trace; trace={}",
                live.trace_summary()
            )
        })
}

fn assert_delivered_submissions_exactly_once(
    live: &mut LiveSession,
    submissions: &[&Submission],
) -> Result<(), String> {
    for submission in submissions {
        if live.tracks_prompt_delivery(submission.mode)
            && delivered_position(live, &submission.id).is_some()
        {
            live.assert_submission_once(submission)?;
        }
    }
    Ok(())
}

fn assert_retired_submission_isolated(
    live: &mut LiveSession,
    old: &Input,
    newer: &Input,
) -> Result<(), String> {
    live.assert_transcript_user_once(&old.marker, old.submission.images.len())?;
    live.assert_submission_once(&newer.submission)?;
    if live.conversation().items.iter().any(|item| {
        item.kind == TranscriptKind::User
            && item.complete_text().contains(&old.marker)
            && item.complete_text().contains(&newer.marker)
    }) {
        return Err(format!(
            "late receipt merged retired and newer inputs; transcript={}",
            live.transcript_summary()
        ));
    }
    Ok(())
}

fn require_late_old_delivery(
    live: &mut LiveSession,
    old: &Input,
    newer: &Input,
) -> Result<(), String> {
    live.wait_for_activity_after(newer.submitted_at, TURN_TIMEOUT, |event| {
        event["type"].as_str() == Some("prompt_delivery")
            && event["submissionId"].as_str() == Some(old.submission.id.as_str())
            && matches!(event["status"].as_str(), Some("accepted" | "delivered"))
    })
    .map_err(|error| {
        format!(
            "E2E_BLOCKED: {} did not expose an old ID-correlated receipt after the new submission began; {error}",
            live.harness()
        )
    })?;
    let response = live
        .wait_for_response(&old.submission.id, RECEIPT_TIMEOUT)
        .map_err(|error| format!("E2E_BLOCKED: old receipt was never correlated: {error}"))?;
    if response.operation() != SessionOperation::Prompt(old.submission.mode) {
        return Err(format!(
            "E2E_BLOCKED: old receipt changed operation after the control race: {:?}",
            response.operation()
        ));
    }
    if let Err(error) = response.result
        && error.kind != SessionResponseErrorKind::DeliveryUnknown
    {
        return Err(format!(
            "E2E_BLOCKED: old receipt became an unrelated terminal error: {error}"
        ));
    }
    live.wait_for_delivery_after(newer.submitted_at, &old.submission.id, TURN_TIMEOUT)
        .map_err(|error| {
            format!(
                "E2E_BLOCKED: {} did not expose old native delivery after the new submission began; {error}",
                live.harness()
            )
        })?;
    live.assert_submission_once(&old.submission)
}
