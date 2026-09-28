use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::{
    SessionEvent,
    extensions::{PromptImage, PromptMode},
};

#[derive(Default)]
pub(super) struct Deliveries(VecDeque<(String, PromptMode, Value)>);

impl Deliveries {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn submitted(&mut self, id: String, mode: PromptMode, text: &str, images: &[PromptImage]) {
        let mut content = vec![json!({"type":"text", "text":text})];
        content.extend(images.iter().map(|image| {
            json!({
                "type":"image", "data":image.data, "mimeType":image.mime_type,
            })
        }));
        self.0.push_back((id, mode, content.into()));
    }

    pub fn reject(&mut self, id: &str) {
        self.0.retain(|(pending, _, _)| pending != id);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn observe(&mut self, event: &Value) -> Option<SessionEvent> {
        let message = &event["message"];
        if message["role"] != "user" {
            return None;
        }
        let content = if let Some(text) = message["content"].as_str() {
            json!([{"type":"text", "text":text}])
        } else {
            message["content"].clone()
        };
        let index = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, (_, _, pending))| pending == &content)
            .min_by_key(|(_, (_, mode, _))| match mode {
                PromptMode::Normal => 0,
                PromptMode::Steer => 1,
                PromptMode::FollowUp => 2,
            })
            .map(|(index, _)| index)?;
        match event["type"].as_str()? {
            "message_start" => Some(SessionEvent::Stderr(String::new())),
            "message_end" => {
                let mut message: crate::DeliveredMessage =
                    serde_json::from_value(message.clone()).ok()?;
                let (id, mode, _) = self.0.remove(index)?;
                message.queued = mode != PromptMode::Normal;
                message.delivery_tracked = true;
                message.prompt_mode = Some(mode);
                Some(SessionEvent::Activity(
                    crate::contract::PromptDelivery {
                        submission_id: id,
                        status: crate::DeliveryStatus::Delivered,
                        message: Some(message),
                    }
                    .into(),
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
