use super::worker::WorkerSnapshot;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use super::caller::WorkerParent;
use super::worker::{WorkerEvent, WorkerSession};
use crate::contract::WorkerStatus;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) enum RunCommand {
    Stop,
    Retire,
}

pub(super) fn spawn(
    id: &str,
    session: Box<dyn WorkerSession>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    slot: super::WorkerSlot,
    parent: Option<WorkerParent>,
    updates: async_channel::Sender<()>,
    cleanup_confirmed: Arc<AtomicBool>,
) -> Result<(mpsc::Sender<RunCommand>, thread::JoinHandle<()>), String> {
    let (commands, receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("farcaster-worker-{id}"))
        .spawn(move || {
            run(
                session,
                receiver,
                snapshot,
                slot,
                parent,
                &updates,
                &cleanup_confirmed,
            )
        })
        .map_err(|error| format!("start worker thread: {error}"))?;
    Ok((commands, handle))
}

fn run(
    mut session: Box<dyn WorkerSession>,
    commands: mpsc::Receiver<RunCommand>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    slot: super::WorkerSlot,
    parent: Option<WorkerParent>,
    updates: &async_channel::Sender<()>,
    cleanup_confirmed: &AtomicBool,
) {
    let (responses, response_rx) = mpsc::channel::<crate::WorkerInputResponse>();
    let mut input_leases = Vec::new();
    let (question_responses, question_response_rx) = mpsc::channel::<crate::QuestionResponse>();
    let mut questions = std::collections::BTreeMap::<String, bool>::new();
    let mut approvals = std::collections::BTreeMap::<String, crate::WorkerInput>::new();
    let mut turn_active = true;
    let error = 'run: loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(RunCommand::Stop) => {
                let _ = session.abort();
                let close_error = session.close().err();
                cleanup_confirmed.store(close_error.is_none(), Ordering::SeqCst);
                slot.release();
                update(&snapshot, |current| {
                    if let Some(error) = close_error {
                        current.status = WorkerStatus::Failed;
                        current.error = Some(format!("worker cleanup failed: {error}"));
                    } else {
                        current.status = WorkerStatus::Stopped;
                        current.error = None;
                    }
                });
                notify(updates);
                return;
            }
            Ok(RunCommand::Retire) => {
                let close_error = session.close().err();
                cleanup_confirmed.store(close_error.is_none(), Ordering::SeqCst);
                slot.release();
                if let Some(error) = close_error {
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::Failed;
                        current.error = Some(format!("worker cleanup failed: {error}"));
                    });
                }
                notify(updates);
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                cleanup_confirmed.store(session.close().is_ok(), Ordering::SeqCst);
                slot.release();
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while let Ok(response) = response_rx.try_recv() {
            let id = response.id.clone();
            if let Err(error) = session.respond(response) {
                break 'run error;
            }
            input_leases.retain(|(input_id, _)| input_id != &id);
            approvals.remove(&id);
            refresh_inputs(&snapshot, &approvals, &questions, turn_active);
            notify(updates);
        }
        while let Ok(response) = question_response_rx.try_recv() {
            let id = response.id.clone();
            if !questions.contains_key(&id) {
                continue;
            }
            if let Err(error) = session.respond_questions(response) {
                break 'run error;
            }
            // Keep the worker resident until the adapter confirms resolution/delivery.
            if let Some(blocking) = questions.get_mut(&id) {
                *blocking = false;
            }
            input_leases.retain(|(input_id, _)| input_id != &id);
            refresh_inputs(&snapshot, &approvals, &questions, turn_active);
            notify(updates);
        }
        while let Some(event) = session.poll() {
            match event {
                WorkerEvent::Started => {
                    turn_active = true;
                    refresh_inputs(&snapshot, &approvals, &questions, turn_active);
                }
                WorkerEvent::Settled { output } => {
                    if std::mem::replace(&mut turn_active, false)
                        && !output.trim().is_empty()
                        && let Some(parent) = &parent
                    {
                        parent.report(output.clone());
                    }
                    // Questions have explicit resolution events and can outlive the main turn.
                    input_leases.retain(|(id, _)| questions.contains_key(id));
                    approvals.clear();
                    slot.release();
                    update(&snapshot, |current| {
                        current.output = Some(output);
                        current.error = None;
                        current.pending_input = None;
                    });
                    refresh_inputs(&snapshot, &approvals, &questions, turn_active);
                    notify(updates);
                }
                WorkerEvent::SessionChanged { locator } => {
                    update(&snapshot, |current| {
                        current.session_locator = Some(locator);
                    });
                    notify(updates);
                }
                WorkerEvent::NeedsInput(input) => {
                    if let Some(parent) = &parent {
                        match super::CallerRegistry::shared().request_child_input(
                            parent,
                            input.clone(),
                            responses.clone(),
                        ) {
                            Ok(lease) => input_leases.push((input.id.clone(), lease)),
                            Err(error) => break 'run error,
                        }
                    }
                    approvals.insert(input.id.clone(), input);
                    refresh_inputs(&snapshot, &approvals, &questions, turn_active);
                    notify(updates);
                }
                WorkerEvent::QuestionsRequested(request) => {
                    if let Some(parent) = &parent {
                        match super::CallerRegistry::shared().request_child_questions(
                            parent,
                            request.clone(),
                            question_responses.clone(),
                        ) {
                            Ok(lease) => input_leases.push((request.id.clone(), lease)),
                            Err(error) => break 'run error,
                        }
                    }
                    questions.insert(request.id, request.blocking);
                    refresh_inputs(&snapshot, &approvals, &questions, turn_active);
                    notify(updates);
                }
                WorkerEvent::InputResolved { id } => {
                    approvals.remove(&id);
                    questions.remove(&id);
                    input_leases.retain(|(input_id, _)| input_id != &id);
                    refresh_inputs(&snapshot, &approvals, &questions, turn_active);
                    notify(updates);
                }
                WorkerEvent::Activity(_) => {}
                WorkerEvent::RequestFailed { operation, error } => {
                    if let Some(parent) = &parent {
                        parent.report(format!("{operation} failed: {error}"));
                    }
                }
                WorkerEvent::PromptDeliveryUnknown { error, .. } => {
                    if let Some(parent) = &parent {
                        parent.report(format!("Input delivery is unknown: {error}"));
                    }
                }
                WorkerEvent::PromptCancelled { .. } => {}
                WorkerEvent::Failed(error) => break 'run error,
            }
        }
    };
    let error = close_failed(&mut *session, &snapshot, error, cleanup_confirmed);
    slot.release();
    if let Some(parent) = &parent {
        parent.report(format!("Worker failed: {error}"));
    }
    notify(updates);
}

fn refresh_inputs(
    snapshot: &Mutex<WorkerSnapshot>,
    approvals: &std::collections::BTreeMap<String, crate::WorkerInput>,
    questions: &std::collections::BTreeMap<String, bool>,
    turn_active: bool,
) {
    update(snapshot, |current| {
        current.pending_input = approvals.values().next().cloned();
        current.status = if !approvals.is_empty()
            || questions.values().any(|blocking| *blocking || !turn_active)
        {
            WorkerStatus::NeedsInput
        } else if turn_active {
            WorkerStatus::Running
        } else {
            WorkerStatus::Idle
        };
    });
}

fn notify(updates: &async_channel::Sender<()>) {
    let _ = updates.try_send(());
}

fn update(snapshot: &Mutex<WorkerSnapshot>, change: impl FnOnce(&mut WorkerSnapshot)) {
    if let Ok(mut snapshot) = snapshot.lock() {
        change(&mut snapshot);
        super::activity::record(&snapshot.project);
    }
}

fn close_failed(
    session: &mut dyn WorkerSession,
    snapshot: &Mutex<WorkerSnapshot>,
    mut error: String,
    cleanup_confirmed: &AtomicBool,
) -> String {
    let close_error = session.close().err();
    cleanup_confirmed.store(close_error.is_none(), Ordering::SeqCst);
    if let Some(close_error) = close_error {
        error.push_str(&format!("; worker cleanup failed: {close_error}"));
    }
    update(snapshot, |current| {
        current.status = WorkerStatus::Failed;
        current.error = Some(error.clone());
        current.pending_input = None;
    });
    error
}
