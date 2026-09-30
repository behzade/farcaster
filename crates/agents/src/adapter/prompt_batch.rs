use std::collections::VecDeque;

use super::{prompt_acknowledgements::PromptAcknowledgements, prompt_input::PromptInput};
use crate::{PromptRejection, WorkerActivity, WorkerEvent};

#[derive(Default)]
enum Delivery {
    #[default]
    Pending,
    Unknown,
    Delivered,
}

#[derive(Default)]
pub(super) struct PromptBatch {
    inputs: Vec<WorkerActivity>,
    delivery: Delivery,
}

impl From<Vec<WorkerActivity>> for PromptBatch {
    fn from(inputs: Vec<WorkerActivity>) -> Self {
        Self {
            inputs,
            delivery: Delivery::Pending,
        }
    }
}

impl From<Vec<PromptInput>> for PromptBatch {
    fn from(inputs: Vec<PromptInput>) -> Self {
        inputs
            .into_iter()
            .map(PromptInput::into_activity)
            .collect::<Vec<_>>()
            .into()
    }
}

impl PromptBatch {
    pub(super) fn first_submission_id(&self) -> Option<&str> {
        self.inputs.first().and_then(submission_id)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }

    pub(super) fn is_delivered(&self) -> bool {
        matches!(self.delivery, Delivery::Delivered)
    }

    pub(super) fn acknowledge(
        &mut self,
        acks: &mut PromptAcknowledgements,
        events: &mut VecDeque<WorkerEvent>,
    ) {
        self.delivery = Delivery::Delivered;
        for input in self.inputs.drain(..) {
            if let Some(id) = submission_id(&input) {
                acks.record(id.to_owned(), Ok(()));
            }
            events.push_back(WorkerEvent::Activity(input));
        }
    }

    pub(super) fn mark_unknown(&mut self, error: &str, events: &mut VecDeque<WorkerEvent>) {
        if !matches!(self.delivery, Delivery::Pending) {
            return;
        }
        self.delivery = Delivery::Unknown;
        for input in &self.inputs {
            if let Some(id) = submission_id(input) {
                events.push_back(WorkerEvent::PromptDeliveryUnknown {
                    submission_id: id.to_owned(),
                    error: error.to_owned(),
                });
            }
        }
    }

    pub(super) fn reject(self, error: PromptRejection, acks: &mut PromptAcknowledgements) {
        for input in self.inputs {
            if let Some(id) = submission_id(&input) {
                acks.record(id.to_owned(), Err(error.clone()));
            }
        }
    }
}

fn submission_id(input: &WorkerActivity) -> Option<&str> {
    match input {
        WorkerActivity::InputDelivered { submission_id, .. } => submission_id.as_deref(),
        _ => None,
    }
}
