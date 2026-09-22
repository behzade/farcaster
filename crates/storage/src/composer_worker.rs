use super::*;
use std::{
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::JoinHandle,
};

const WRITE_DELAY: Duration = Duration::from_millis(250);

enum PersistenceCommand {
    Save(ComposerRecord),
    Delete(String),
    Shutdown,
}

impl PersistenceCommand {
    fn target(&self) -> Option<&str> {
        match self {
            Self::Save(record) => Some(&record.target),
            Self::Delete(target) => Some(target),
            Self::Shutdown => None,
        }
    }
}

pub struct ComposerPersistenceWorker {
    sender: Sender<PersistenceCommand>,
    worker: Option<JoinHandle<()>>,
}

impl ComposerPersistenceWorker {
    pub fn spawn(store: Result<SharedStateStore, String>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("farcaster-composer-state".into())
            .spawn(move || {
                let store = match store {
                    Ok(store) => store,
                    Err(error) => {
                        log::error!("Open composer state: {error}");
                        return;
                    }
                };
                let mut pending: Vec<PersistenceCommand> = Vec::new();
                let mut deadline: Option<Instant> = None;
                loop {
                    let received = match deadline {
                        Some(due) => {
                            receiver.recv_timeout(due.saturating_duration_since(Instant::now()))
                        }
                        None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match received {
                        Ok(PersistenceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                            flush_shared(&store, &mut pending);
                            break;
                        }
                        Ok(command) => {
                            pending.retain(|previous| previous.target() != command.target());
                            pending.push(command);
                            deadline.get_or_insert_with(|| Instant::now() + WRITE_DELAY);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    if deadline.is_some_and(|due| Instant::now() >= due) {
                        flush_shared(&store, &mut pending);
                        deadline = (!pending.is_empty()).then(|| Instant::now() + WRITE_DELAY);
                    }
                }
            })
            .inspect_err(|error| {
                log::error!("Start composer persistence: {error}");
            })
            .ok();
        Self { sender, worker }
    }
}

impl sessions::ComposerPersistence<ComposerAttachment> for ComposerPersistenceWorker {
    fn save(&self, record: ComposerRecord) {
        let _ = self.sender.send(PersistenceCommand::Save(record));
    }

    fn delete(&self, target: String) {
        let _ = self.sender.send(PersistenceCommand::Delete(target));
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

fn flush_shared(store: &SharedStateStore, pending: &mut Vec<PersistenceCommand>) {
    if let Err(error) = store.with(|store| {
        flush(store, pending);
        Ok(())
    }) {
        log::error!("Save composer state: {error}");
    }
}

fn flush(store: &StateStore, pending: &mut Vec<PersistenceCommand>) {
    let mut completed = 0;
    for command in pending.iter() {
        let result = match command {
            PersistenceCommand::Save(record) => store.save_composer_session(record),
            PersistenceCommand::Delete(target) => store.delete_composer_session(target),
            PersistenceCommand::Shutdown => unreachable!("shutdown is not queued"),
        };
        if let Err(error) = result {
            log::error!("Save composer state: {error}");
            break;
        }
        completed += 1;
    }
    pending.drain(..completed);
}

#[cfg(test)]
#[path = "composer_persistence_tests.rs"]
mod tests;
