use std::path::PathBuf;

use super::super::contract::QueuedPrompt;

pub trait PromptStore {
    fn has_queued_for(&self, paths: &[PathBuf]) -> Result<bool, String>;
    fn queued(&self) -> Result<Vec<QueuedPrompt>, String>;
    fn begin(&self, id: i64) -> Result<(), String>;
}

pub fn has_queued_for(store: &impl PromptStore, paths: &[PathBuf]) -> Result<bool, String> {
    store.has_queued_for(paths)
}

pub fn queued(store: &impl PromptStore) -> Result<Vec<QueuedPrompt>, String> {
    store.queued()
}

pub fn begin(store: &impl PromptStore, id: i64) -> Result<(), String> {
    store.begin(id)
}
