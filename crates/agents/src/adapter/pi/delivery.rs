use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::{
    SessionEvent,
    extensions::{PromptImage, PromptMode},
};

#[derive(Default)]
pub(super) struct Deliveries(VecDeque<(String, PromptMode, Value)>);

impl Deliveries {
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
        // Pi has no client ID on user events. Match payloads (including images)
        // in Pi's normal/steer/follow-up order, FIFO within each mode.
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
                let (id, mode, _) = self.0.remove(index)?;
                let mut message = message.clone();
                message["queued"] = (mode != PromptMode::Normal).into();
                message["deliveryTracked"] = true.into();
                message["promptMode"] = match mode {
                    PromptMode::Normal => "normal",
                    PromptMode::Steer => "steer",
                    PromptMode::FollowUp => "follow_up",
                }
                .into();
                Some(SessionEvent::Activity(
                    json!({
                        "type":"prompt_delivery", "submissionId":id,
                        "status":"delivered", "message":message,
                    })
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
