use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

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
    parent_worker_id: Option<String>,
    updates: async_channel::Sender<()>,
) -> Result<(mpsc::Sender<RunCommand>, thread::JoinHandle<()>), String> {
    let (commands, receiver) = mpsc::channel();
    let worker_id = id.to_owned();
    let handle = thread::Builder::new()
        .name(format!("farcaster-worker-{id}"))
        .spawn(move || {
            run(
                session,
                receiver,
                snapshot,
                &worker_id,
                parent_worker_id.as_deref(),
                &updates,
            )
        })
        .map_err(|error| format!("start worker thread: {error}"))?;
    Ok((commands, handle))
}

fn run(
    mut session: Box<dyn WorkerSession>,
    commands: mpsc::Receiver<RunCommand>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
    worker_id: &str,
    parent_worker_id: Option<&str>,
    updates: &async_channel::Sender<()>,
) {
    loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(RunCommand::Stop) => {
                let _ = session.abort();
                let close_error = session.close().err();
                update(&snapshot, |current| {
                    current.status = WorkerStatus::Stopped;
                    current.error = close_error;
                });
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = session.close();
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
                    let parent_output = output.clone();
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::Idle;
                        current.output = Some(output);
                        current.error = None;
                        current.pending_input = None;
                    });
                    notify(updates);
                    report_to_parent(worker_id, parent_worker_id, parent_output);
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
                    let parent_output = format!("Worker failed: {error}");
                    close_failed(&mut *session, &snapshot, error);
                    notify(updates);
                    report_to_parent(worker_id, parent_worker_id, parent_output);
                    return;
                }
            }
        }
    }
}

fn notify(updates: &async_channel::Sender<()>) {
    let _ = updates.try_send(());
}

fn report_to_parent(worker_id: &str, parent_worker_id: Option<&str>, output: String) {
    if parent_worker_id.is_none() {
        return;
    }
    let message = if output.trim().is_empty() {
        "Worker completed without output.".to_owned()
    } else {
        output
    };
    if let Err(error) = crate::modules::agents::core::CallerRegistry::shared()
        .report_from_worker(worker_id, message)
    {
        zlog::warn!("Failed to deliver child worker {worker_id} result: {error}");
    }
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
) {
    if let Err(close_error) = session.close() {
        error.push_str(&format!("; worker cleanup failed: {close_error}"));
    }
    update(snapshot, |current| {
        current.status = WorkerStatus::Failed;
        current.error = Some(error);
        current.pending_input = None;
    });
}
