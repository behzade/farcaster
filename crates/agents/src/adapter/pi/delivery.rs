use std::{borrow::Cow, collections::VecDeque};

use serde::Deserialize as _;
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
        if self.is_empty() {
            return None;
        }
        let message = &event["message"];
        if message["role"] != "user" {
            return None;
        }
        let content = if let Some(text) = message["content"].as_str() {
            Cow::Owned(json!([{"type":"text", "text":text}]))
        } else {
            Cow::Borrowed(&message["content"])
        };
        let index = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, (_, _, pending))| pending == content.as_ref())
            .min_by_key(|(_, (_, mode, _))| match mode {
                PromptMode::Normal => 0,
                PromptMode::Steer => 1,
                PromptMode::FollowUp => 2,
            })
            .map(|(index, _)| index)?;
        match event["type"].as_str()? {
            "message_start" => Some(SessionEvent::Stderr(String::new())),
            "message_end" => {
                let mut message = crate::DeliveredMessage::deserialize(message).ok()?;
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
