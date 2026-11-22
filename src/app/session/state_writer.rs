use crate::{
    app::{FarcasterApp, persistence},
    sessions::DraftSession,
    storage::SessionStateWriter as StorageWriter,
};

pub(in crate::app) struct SessionStateWriter {
    worker: StorageWriter,
    errors: [Option<SaveFailure>; 2],
}

struct SaveFailure {
    detail: String,
    dismissed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum PersistenceSource {
    Session,
    Composer,
}

impl std::ops::Deref for SessionStateWriter {
    type Target = StorageWriter;

    fn deref(&self) -> &Self::Target {
        &self.worker
    }
}

impl SessionStateWriter {
    pub(in crate::app) fn new(cx: &mut gpui::Context<FarcasterApp>) -> Self {
        let (writer, updates) = StorageWriter::spawn(persistence::shared);
        observe_updates(PersistenceSource::Session, updates, cx);
        Self {
            worker: writer,
            errors: Default::default(),
        }
    }

    #[cfg(test)]
    fn spawn(
        open: impl Fn() -> Result<crate::storage::SharedStateStore, String> + Send + 'static,
    ) -> (Self, async_channel::Receiver<Result<(), String>>) {
        let (writer, updates) = StorageWriter::spawn(open);
        (
            Self {
                worker: writer,
                errors: Default::default(),
            },
            updates,
        )
    }

    pub(in crate::app) fn warning(&self) -> Option<(PersistenceSource, &str)> {
        [PersistenceSource::Composer, PersistenceSource::Session]
            .into_iter()
            .find_map(|source| {
                let failure = self.errors[source as usize].as_ref()?;
                (!failure.dismissed).then_some((source, failure.detail.as_str()))
            })
    }

    pub(in crate::app) fn dismiss_warning(&mut self, source: PersistenceSource, detail: &str) {
        if let Some(failure) = &mut self.errors[source as usize]
            && failure.detail == detail
        {
            failure.dismissed = true;
        }
    }

    fn update_error(&mut self, source: PersistenceSource, result: Result<(), String>) {
        let current = &mut self.errors[source as usize];
        match result {
            Err(detail)
                if current
                    .as_ref()
                    .is_none_or(|failure| failure.detail != detail) =>
            {
                zlog::warn!("Failed to persist session or composer state: {detail}");
                *current = Some(SaveFailure {
                    detail,
                    dismissed: false,
                });
            }
            Err(_) => {}
            Ok(()) => *current = None,
        }
    }
}

type DraftIdentity = Result<crate::sessions::AppSessionId, String>;
pub(in crate::app) type DraftReceipts =
    async_channel::Sender<(DraftSession, async_channel::Receiver<DraftIdentity>)>;

pub(in crate::app) fn observe_draft_receipts(
    cx: &mut gpui::Context<FarcasterApp>,
) -> DraftReceipts {
    let (sender, receipts) =
        async_channel::unbounded::<(DraftSession, async_channel::Receiver<DraftIdentity>)>();
    cx.spawn(async move |weak, cx| {
        while let Ok((snapshot, receipt)) = receipts.recv().await {
            let result = receipt
                .recv()
                .await
                .unwrap_or_else(|_| Err("Session state writer stopped before saving".into()));
            if weak
                .update(cx, |app, cx| {
                    let id = &snapshot.id;
                    let latest = app.sessions.allocating_drafts.remove(id);
                    match result {
                        Ok(identity) => {
                            app.adopt_draft_identity(id.clone(), identity);
                            if let Some(mut latest) = latest
                                && latest != snapshot
                                && app.sessions.draft_session_ids.get(id) == Some(&identity.get())
                            {
                                latest.app_session_id = identity.get();
                                if let Err(error) = app.sessions.writer.save_draft(latest) {
                                    app.sessions.error = Some(error);
                                }
                            }
                        }
                        Err(error) if app.sessions.pending_draft_saves.contains(id) => {
                            app.sessions.error = Some(error);
                        }
                        Err(_) => return,
                    }
                    app.notify_session_rail(cx);
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
    sender
}

pub(in crate::app) fn observe_updates(
    source: PersistenceSource,
    updates: async_channel::Receiver<Result<(), String>>,
    cx: &mut gpui::Context<FarcasterApp>,
) {
    cx.spawn(async move |weak, cx| {
        while let Ok(result) = updates.recv().await {
            if weak
                .update(cx, |app, cx| {
                    app.sessions.writer.update_error(source, result);
                    app.notify_session_rail_shell(cx);
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
#[path = "state_writer_tests.rs"]
mod tests;
