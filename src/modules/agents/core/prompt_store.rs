use std::path::{Path, PathBuf};

use super::super::contract::{
    QueuedPrompt,
    extensions::{PromptImage, PromptMode},
};

pub(crate) trait PromptStore {
    fn has_queued_for(&self, paths: &[PathBuf]) -> Result<bool, String>;

    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        images: &[PromptImage],
    ) -> Result<i64, String>;

    #[allow(clippy::too_many_arguments)]
    fn enqueue_with_presentation(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        display_message: Option<&str>,
        invocation: Option<&str>,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        let _ = (display_message, invocation);
        self.enqueue(target, harness, project, session, mode, message, images)
    }

    fn queued(&self) -> Result<Vec<QueuedPrompt>, String>;
    fn complete(&mut self, id: i64, target: &str, session: Option<&Path>) -> Result<(), String>;
    fn begin(&self, id: i64) -> Result<(), String>;
    fn fail(&self, id: i64, error: &str) -> Result<(), String>;
}

pub(crate) fn has_queued_for(store: &impl PromptStore, paths: &[PathBuf]) -> Result<bool, String> {
    store.has_queued_for(paths)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_with_presentation(
    store: &impl PromptStore,
    target: &str,
    harness: &str,
    project: &Path,
    session: Option<&Path>,
    mode: PromptMode,
    message: &str,
    display_message: Option<&str>,
    invocation: Option<&str>,
    images: &[PromptImage],
) -> Result<i64, String> {
    store.enqueue_with_presentation(
        target,
        harness,
        project,
        session,
        mode,
        message,
        display_message,
        invocation,
        images,
    )
}

pub(crate) fn queued(store: &impl PromptStore) -> Result<Vec<QueuedPrompt>, String> {
    store.queued()
}

pub(crate) fn complete(
    store: &mut impl PromptStore,
    id: i64,
    target: &str,
    session: Option<&Path>,
) -> Result<(), String> {
    store.complete(id, target, session)
}

pub(crate) fn begin(store: &impl PromptStore, id: i64) -> Result<(), String> {
    store.begin(id)
}

pub(crate) fn fail(store: &impl PromptStore, id: i64, error: &str) -> Result<(), String> {
    store.fail(id, error)
}
