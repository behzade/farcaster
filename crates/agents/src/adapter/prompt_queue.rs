use std::collections::VecDeque;

use super::prompt_input::PromptInput;
use crate::WorkerSendMode;

pub(super) trait QueuedPrompt {
    fn input(&self) -> &PromptInput;
}

impl QueuedPrompt for PromptInput {
    fn input(&self) -> &PromptInput {
        self
    }
}

pub(super) struct PromptQueue<T>(VecDeque<T>);

impl<T> Default for PromptQueue<T> {
    fn default() -> Self {
        Self(VecDeque::new())
    }
}

impl<T> PromptQueue<T> {
    pub(super) fn push(&mut self, input: T) {
        self.0.push_back(input);
    }

    pub(super) fn pop(&mut self) -> Option<T> {
        self.0.pop_front()
    }

    pub(super) fn take_all(&mut self) -> Vec<T> {
        std::mem::take(&mut self.0).into()
    }

    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.0.iter()
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T: QueuedPrompt> PromptQueue<T> {
    pub(super) fn take_steers(&mut self) -> Vec<T> {
        let (steers, retained) = self
            .0
            .drain(..)
            .partition(|prompt| prompt.input().mode == WorkerSendMode::Steer);
        self.0 = retained;
        steers.into()
    }

    pub(super) fn cancel(&mut self, id: &str) -> Option<T> {
        let index = self
            .0
            .iter()
            .position(|prompt| prompt.input().submission_id.as_deref() == Some(id))?;
        self.0.remove(index)
    }
}
