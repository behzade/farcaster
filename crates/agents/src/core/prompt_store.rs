use crate::Backend;
use std::path::{Path, PathBuf};

use super::super::contract::{
    QueuedPrompt,
    extensions::{PromptImage, PromptMode},
};

pub trait PromptStore {
    fn has_queued_for(&self, paths: &[PathBuf]) -> Result<bool, String>;

    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &self,
        target: &str,
        harness: Backend,
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
        harness: Backend,
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
    fn begin(&self, id: i64) -> Result<(), String>;
}

pub fn has_queued_for(store: &impl PromptStore, paths: &[PathBuf]) -> Result<bool, String> {
    store.has_queued_for(paths)
}

#[allow(clippy::too_many_arguments)]
pub fn enqueue_with_presentation(
    store: &impl PromptStore,
    target: &str,
    harness: Backend,
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

pub fn queued(store: &impl PromptStore) -> Result<Vec<QueuedPrompt>, String> {
    store.queued()
}

pub fn begin(store: &impl PromptStore, id: i64) -> Result<(), String> {
    store.begin(id)
}
