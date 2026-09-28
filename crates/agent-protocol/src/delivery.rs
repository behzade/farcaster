use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::extensions::PromptMode;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    Accepted,
    Delivered,
    Unknown,
    Rejected,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename = "prompt_delivery", rename_all = "camelCase")]
pub struct PromptDelivery {
    #[serde(deserialize_with = "nonempty_id")]
    pub submission_id: String,
    pub status: DeliveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<DeliveredMessage>,
}

fn nonempty_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let id = String::deserialize(deserializer)?;
    if id.is_empty() {
        return Err(serde::de::Error::custom("empty prompt submission ID"));
    }
    Ok(id)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeliveredRole {
    User,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeliveredMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<DeliveredRole>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub content: Value,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub delivery_tracked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_mode: Option<PromptMode>,
    #[serde(flatten)]
    pub metadata: Map<String, Value>,
}

impl DeliveredMessage {
    pub fn user(content: Value, mode: PromptMode, delivery_tracked: bool) -> Self {
        Self {
            role: Some(DeliveredRole::User),
            content,
            queued: mode != PromptMode::Normal,
            delivery_tracked,
            prompt_mode: Some(mode),
            metadata: Map::new(),
        }
    }

    pub fn value(&self) -> Value {
        serde_json::to_value(self).expect("delivery message contains only JSON values")
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
