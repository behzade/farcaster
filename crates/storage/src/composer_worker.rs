use super::*;
use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::JoinHandle,
};

const WRITE_DELAY: Duration = Duration::from_millis(250);

enum PersistenceCommand {
    Save(ComposerRecord),
    Delete(String),
    Flush(async_channel::Sender<Result<(), String>>),
    Shutdown,
}

impl PersistenceCommand {
    fn target(&self) -> Option<&str> {
        match self {
            Self::Save(record) => Some(&record.target),
            Self::Delete(target) => Some(target),
            Self::Flush(_) | Self::Shutdown => None,
        }
    }
}

pub struct ComposerPersistenceWorker {
    sender: Sender<PersistenceCommand>,
    updates: async_channel::Sender<Result<(), String>>,
    revision: Cell<u64>,
    worker: Option<JoinHandle<()>>,
}

impl ComposerPersistenceWorker {
    pub fn spawn(
        open: impl Fn() -> Result<SharedStateStore, String> + Send + 'static,
    ) -> (Self, async_channel::Receiver<Result<(), String>>) {
        let (sender, receiver) = mpsc::channel();
        let (updates, errors) = async_channel::unbounded();
        let worker_updates = updates.clone();
        let worker = std::thread::Builder::new()
            .name("farcaster-composer-state".into())
            .spawn(move || run(receiver, worker_updates, open))
            .inspect_err(|error| {
                let _ = updates.send_blocking(Err(format!("Start composer persistence: {error}")));
            })
            .ok();
        (
            Self {
                sender,
                updates,
                revision: Cell::new(0),
                worker,
            },
            errors,
        )
    }

    fn send(&self, command: PersistenceCommand) -> Result<(), String> {
        let changes_state = matches!(
            &command,
            PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
        );
        self.sender.send(command).map_err(|_| {
            let error = "Composer persistence writer stopped".to_owned();
            let _ = self.updates.send_blocking(Err(error.clone()));
            error
        })?;
        if changes_state {
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        Ok(())
    }
}

impl sessions::ComposerPersistence<ComposerAttachment> for ComposerPersistenceWorker {
    fn save(&self, record: ComposerRecord) {
        let _ = self.send(PersistenceCommand::Save(record));
    }

    fn delete(&self, target: String) {
        let _ = self.send(PersistenceCommand::Delete(target));
    }

    fn flush(&self) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
        let (sender, receiver) = async_channel::bounded(1);
        let sent = self.send(PersistenceCommand::Flush(sender));
        Box::pin(async move {
            sent?;
            receiver
                .recv()
                .await
                .map_err(|_| "Composer persistence writer stopped before saving".to_owned())?
        })
    }

    fn revision(&self) -> u64 {
        self.revision.get()
    }
}

impl Drop for ComposerPersistenceWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(PersistenceCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    receiver: mpsc::Receiver<PersistenceCommand>,
    updates: async_channel::Sender<Result<(), String>>,
    open: impl Fn() -> Result<SharedStateStore, String>,
) {
    let mut pending = Vec::new();
    let mut deadline: Option<Instant> = None;
    let mut previous_error = None;
    loop {
        let received = match deadline {
            Some(due) => receiver.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let mut barrier = None;
        let mut shutdown = false;
        match received {
            Ok(PersistenceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                shutdown = true
            }
            Ok(PersistenceCommand::Flush(sender)) => barrier = Some(sender),
            Ok(command) => {
                pending
                    .retain(|previous: &PersistenceCommand| previous.target() != command.target());
                pending.push(command);
                deadline.get_or_insert_with(|| Instant::now() + WRITE_DELAY);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if shutdown || barrier.is_some() || deadline.is_some_and(|due| Instant::now() >= due) {
            let result = if pending.is_empty() {
                Ok(())
            } else {
                open().and_then(|store| store.with(|store| flush(store, &mut pending)))
            };
            match &result {
                Ok(()) => {
                    if previous_error.take().is_some() {
                        let _ = updates.send_blocking(Ok(()));
                    }
                }
                Err(error) => {
                    if previous_error.as_ref() != Some(error) {
                        previous_error = Some(error.clone());
                        let _ = updates.send_blocking(Err(error.clone()));
                    }
                }
            }
            if let Some(barrier) = barrier {
                let _ = barrier.send_blocking(result.clone());
            }
            if shutdown {
                if let Err(error) = result {
                    log::error!("Save composer state at shutdown: {error}");
                }
                break;
            }
            deadline = (!pending.is_empty()).then(|| Instant::now() + WRITE_DELAY);
        }
    }
}

fn flush(store: &StateStore, pending: &mut Vec<PersistenceCommand>) -> Result<(), String> {
    let mut completed = 0;
    let result = pending.iter().try_for_each(|command| {
        match command {
            PersistenceCommand::Save(record) => store.save_composer_session(record),
            PersistenceCommand::Delete(target) => store.delete_composer_session(target),
            PersistenceCommand::Flush(_) | PersistenceCommand::Shutdown => {
                unreachable!("barriers are not queued")
            }
        }?;
        completed += 1;
        Ok(())
    });
    pending.drain(..completed);
    result
}

#[cfg(test)]
#[path = "composer_persistence_tests.rs"]
mod tests;
