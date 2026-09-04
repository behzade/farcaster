use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use super::caller::WorkerParent;
use super::worker::{WorkerEvent, WorkerSession};
use crate::modules::agents::contract::{WorkerSnapshot, WorkerStatus};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) enum RunCommand {
    Stop,
}

pub(super) fn spawn(
    id: &str,
    session: Box<dyn WorkerSession>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    slot: super::WorkerSlot,
    parent: Option<WorkerParent>,
    updates: async_channel::Sender<()>,
) -> Result<(mpsc::Sender<RunCommand>, thread::JoinHandle<()>), String> {
    let (commands, receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("farcaster-worker-{id}"))
        .spawn(move || run(session, receiver, snapshot, slot, parent, &updates))
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
) {
    loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(RunCommand::Stop) => {
                let _ = session.abort();
                let close_error = session.close().err();
                slot.release();
                update(&snapshot, |current| {
                    current.status = WorkerStatus::Stopped;
                    current.error = close_error;
                });
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = session.close();
                slot.release();
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while let Some(event) = session.poll() {
            match event {
                WorkerEvent::Started => update(&snapshot, |current| {
                    current.status = WorkerStatus::Running;
                }),
                WorkerEvent::Settled { output } => {
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
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::NeedsInput;
                        current.pending_input = Some(input);
                    });
                    notify(updates);
                }
                WorkerEvent::Activity(_) => {}
                WorkerEvent::Failed(error) => {
                    let error = close_failed(&mut *session, &snapshot, error);
                    slot.release();
                    if let Some(parent) = &parent {
                        parent.report_failure(&error);
                    }
                    notify(updates);
                    return;
                }
            }
        }
    }
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
) -> String {
    if let Err(close_error) = session.close() {
        error.push_str(&format!("; worker cleanup failed: {close_error}"));
    }
    update(snapshot, |current| {
        current.status = WorkerStatus::Failed;
        current.error = Some(error.clone());
        current.pending_input = None;
    });
    error
}
