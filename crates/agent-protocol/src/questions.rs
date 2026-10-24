use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QuestionRequest {
    /// Correlates answers and explicit resolution, independently of turn completion.
    pub id: String,
    pub blocking: bool,
    pub questions: Vec<Question>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
    pub allow_free_text: bool,
    pub secret: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QuestionResponse {
    pub id: String,
    pub answers: BTreeMap<String, Vec<String>>,
    pub cancelled: bool,
}
