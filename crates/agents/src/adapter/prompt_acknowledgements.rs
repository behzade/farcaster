use std::collections::{HashSet, VecDeque};

use crate::PromptRejection;

type Acknowledgement = (String, Result<(), PromptRejection>);

// An adapter may resolve admission before it can prove delivery to the model.
#[derive(Default)]
pub(super) struct PromptAcknowledgements {
    resolved: HashSet<String>,
    ready: VecDeque<Acknowledgement>,
    deferred: VecDeque<Acknowledgement>,
}

impl PromptAcknowledgements {
    pub(super) fn contains(&self, id: &str) -> bool {
        self.resolved.contains(id)
    }

    pub(super) fn record(&mut self, id: String, result: Result<(), PromptRejection>) {
        if self.resolved.insert(id.clone()) {
            self.ready.push_back((id, result));
        }
    }

    pub(super) fn defer(&mut self, id: String, result: Result<(), PromptRejection>) {
        if self.resolved.insert(id.clone()) {
            self.deferred.push_back((id, result));
        }
    }

    pub(super) fn pop(&mut self) -> Option<Acknowledgement> {
        self.ready.pop_front()
    }

    pub(super) fn release_next(&mut self) -> Option<Acknowledgement> {
        self.deferred.pop_front()
    }

    pub(super) fn has_deferred(&self) -> bool {
        !self.deferred.is_empty()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.deferred.is_empty()
    }
}
