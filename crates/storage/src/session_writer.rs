use std::{cell::Cell, sync::mpsc, time::Duration};

use crate::{
    SessionStateChanges, SharedStateStore,
    projects::ProjectList,
    sessions::{DraftSession, SessionFolders},
};

enum Command {
    Draft(DraftSession),
    Remove(String),
    Projects(ProjectList),
    Folders(SessionFolders),
    Flush(async_channel::Sender<Result<(), String>>),
    Shutdown,
}

/// Writes session changes in atomic batches. Flush before process exit;
/// dropping the handle queues one final attempt without blocking the caller.
pub struct SessionStateWriter {
    sender: mpsc::Sender<Command>,
    revision: Cell<u64>,
}

impl SessionStateWriter {
    pub fn spawn(
        open: impl Fn() -> Result<SharedStateStore, String> + Send + 'static,
    ) -> (Self, async_channel::Receiver<Result<(), String>>) {
        let (sender, receiver) = mpsc::channel();
        let (updates, errors) = async_channel::unbounded();
        let failure = updates.clone();
        std::thread::Builder::new()
            .name("farcaster-session-state".into())
            .spawn(move || run(receiver, updates, open))
            .map_err(|error| {
                let _ = failure.send_blocking(Err(format!("Start session state writer: {error}")));
            })
            .ok();
        (
            Self {
                sender,
                revision: Cell::new(0),
            },
            errors,
        )
    }

    fn send(&self, command: Command) -> Result<(), String> {
        let changes_state = !matches!(&command, Command::Flush(_) | Command::Shutdown);
        self.sender
            .send(command)
            .map_err(|_| "Session state writer stopped".to_owned())?;
        if changes_state {
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn save_draft(&self, draft: DraftSession) -> Result<(), String> {
        self.send(Command::Draft(draft))
    }

    pub fn remove_draft(&self, id: String) -> Result<(), String> {
        self.send(Command::Remove(id))
    }

    pub fn save_projects(&self, projects: ProjectList) -> Result<(), String> {
        self.send(Command::Projects(projects))
    }

    pub fn save_folders(&self, folders: SessionFolders) -> Result<(), String> {
        self.send(Command::Folders(folders))
    }

    pub fn flush(&self) -> impl Future<Output = Result<(), String>> + use<> {
        let (sender, receiver) = async_channel::bounded(1);
        let sent = self.send(Command::Flush(sender));
        async move {
            sent?;
            receiver
                .recv()
                .await
                .map_err(|_| "Session state writer stopped before saving".to_owned())?
        }
    }
}

impl Drop for SessionStateWriter {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
    }
}

fn run(
    receiver: mpsc::Receiver<Command>,
    updates: async_channel::Sender<Result<(), String>>,
    open: impl Fn() -> Result<SharedStateStore, String>,
) {
    let mut pending = SessionStateChanges::default();
    let mut previous_error = None;
    loop {
        let received = if pending.is_empty() {
            receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            receiver.recv_timeout(Duration::from_millis(250))
        };
        let mut command = match received {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Command::Shutdown),
        };
        let mut barrier = None;
        let mut shutdown = false;
        while let Some(next) = command {
            match next {
                Command::Draft(draft) => {
                    pending.drafts.insert(draft.id.clone(), Some(draft));
                }
                Command::Remove(id) => {
                    pending.drafts.insert(id, None);
                }
                Command::Projects(projects) => pending.projects = Some(projects),
                Command::Folders(folders) => pending.folders = Some(folders),
                Command::Flush(sender) => {
                    barrier = Some(sender);
                    break;
                }
                Command::Shutdown => {
                    shutdown = true;
                    break;
                }
            }
            command = receiver.try_recv().ok();
        }
        let result = if pending.is_empty() {
            Ok(())
        } else {
            open().and_then(|store| store.with(|store| store.save_session_changes(&pending)))
        };
        match &result {
            Ok(()) => {
                pending = SessionStateChanges::default();
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
                log::error!("Save session state at shutdown: {error}");
            }
            break;
        }
    }
}

#[cfg(test)]
#[path = "session_writer_tests.rs"]
mod tests;
