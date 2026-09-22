//! Session-local model receipt deduplication shared by prompt adapters.
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct IdempotencyBookkeeping {
    reached_model: HashSet<String>,
}

impl IdempotencyBookkeeping {
    pub(super) fn contains(&self, id: &str) -> bool {
        self.reached_model.contains(id)
    }

    pub(super) fn record_reached_model(&mut self, id: String) {
        self.reached_model.insert(id);
    }
}
