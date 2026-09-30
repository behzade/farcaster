use serde_json::json;

use crate::{
    DeliveredMessage, DeliveryStatus, WorkerActivity, WorkerSendMode,
    contract::PromptDelivery,
    extensions::{PromptImage, PromptMode},
};

#[derive(Clone)]
pub(super) struct PromptInput {
    pub(super) submission_id: Option<String>,
    pub(super) mode: WorkerSendMode,
    pub(super) message: String,
    pub(super) images: Vec<PromptImage>,
}

impl PromptInput {
    pub(super) fn receipt(&self, status: DeliveryStatus) -> PromptDelivery {
        let mut content = vec![json!({"type":"text", "text":self.message})];
        content.extend(
            self.images.iter().map(
                |image| json!({"type":"image", "data":image.data, "mimeType":image.mime_type}),
            ),
        );
        PromptDelivery {
            submission_id: self.submission_id.clone().expect("tracked prompt identity"),
            status,
            message: Some(DeliveredMessage::user(
                content.into(),
                prompt_mode(self.mode),
                true,
            )),
        }
    }

    pub(super) fn into_activity(self) -> WorkerActivity {
        WorkerActivity::InputDelivered {
            submission_id: self.submission_id,
            mode: self.mode,
            message: self.message,
            images: self.images,
        }
    }
}

pub(super) const fn prompt_mode(mode: WorkerSendMode) -> PromptMode {
    match mode {
        WorkerSendMode::Prompt => PromptMode::Normal,
        WorkerSendMode::Queue => PromptMode::FollowUp,
        WorkerSendMode::Steer => PromptMode::Steer,
    }
}

pub(super) const fn worker_mode(mode: PromptMode) -> WorkerSendMode {
    match mode {
        PromptMode::Normal => WorkerSendMode::Prompt,
        PromptMode::FollowUp => WorkerSendMode::Queue,
        PromptMode::Steer => WorkerSendMode::Steer,
    }
}
