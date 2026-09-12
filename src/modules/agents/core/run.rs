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
use crate::modules::agents::contract::{WorkerSnapshot, WorkerStatus};

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
    let (responses, response_rx) = mpsc::channel::<crate::agents::WorkerInputResponse>();
    let mut input_leases = Vec::new();
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
            update(&snapshot, |current| {
                if current
                    .pending_input
                    .as_ref()
                    .is_some_and(|input| input.id == id)
                {
                    current.pending_input = None;
                }
                if input_leases.is_empty() {
                    current.status = WorkerStatus::Running;
                }
            });
            notify(updates);
        }
        while let Some(event) = session.poll() {
            match event {
                WorkerEvent::Started => {
                    turn_active = true;
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::Running;
                    });
                }
                WorkerEvent::Settled { output } => {
                    if std::mem::replace(&mut turn_active, false)
                        && !output.trim().is_empty()
                        && let Some(parent) = &parent
                    {
                        parent.report(output.clone());
                    }
                    input_leases.clear();
                    slot.release();
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::Idle;
                        current.output = Some(output);
                        current.error = None;
                        current.pending_input = None;
                    });
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
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::NeedsInput;
                        current.pending_input = Some(input);
                    });
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

fn notify(updates: &async_channel::Sender<()>) {
    let _ = updates.try_send(());
}

fn update(snapshot: &Mutex<WorkerSnapshot>, change: impl FnOnce(&mut WorkerSnapshot)) {
    if let Ok(mut snapshot) = snapshot.lock() {
        change(&mut snapshot);
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
