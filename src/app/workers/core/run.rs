use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use super::super::{WorkerInputResponse, WorkerSnapshot, WorkerStatus};
use crate::agents::{WorkerEvent, WorkerSendMode, WorkerSession};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) enum RunCommand {
    Send {
        message: String,
        mode: WorkerSendMode,
    },
    Respond(WorkerInputResponse),
    Stop,
}

pub(super) fn spawn(
    id: &str,
    session: Box<dyn WorkerSession>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
) -> Result<(mpsc::Sender<RunCommand>, thread::JoinHandle<()>), String> {
    let (commands, receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name(format!("farcaster-worker-{id}"))
        .spawn(move || run(session, receiver, snapshot))
        .map_err(|error| format!("start worker thread: {error}"))?;
    Ok((commands, handle))
}

fn run(
    mut session: Box<dyn WorkerSession>,
    commands: mpsc::Receiver<RunCommand>,
    snapshot: Arc<Mutex<WorkerSnapshot>>,
) {
    loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(RunCommand::Send { message, mode }) => {
                if let Err(error) = session.send(message, mode) {
                    close_failed(&mut *session, &snapshot, error);
                    return;
                }
            }
            Ok(RunCommand::Respond(response)) => {
                if let Err(error) = session.respond(response) {
                    close_failed(&mut *session, &snapshot, error);
                    return;
                }
            }
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
                WorkerEvent::Settled { output } => update(&snapshot, |current| {
                    current.status = WorkerStatus::Idle;
                    current.output = Some(output);
                    current.error = None;
                    current.pending_input = None;
                }),
                WorkerEvent::SessionChanged { locator } => update(&snapshot, |current| {
                    current.session_locator = Some(locator);
                }),
                WorkerEvent::NeedsInput(input) => {
                    update(&snapshot, |current| {
                        current.status = WorkerStatus::NeedsInput;
                        current.pending_input = Some(input);
                    });
                }
                WorkerEvent::Failed(error) => {
                    close_failed(&mut *session, &snapshot, error);
                    return;
                }
            }
        }
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
