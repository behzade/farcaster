use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Question, QuestionOption, QuestionRequest, QuestionResponse};

use super::contract::CodexRequestId;

pub(super) struct PendingQuestion {
    pub source: QuestionSource,
    pub thread_id: String,
    pub turn_id: String,
    pub request: QuestionRequest,
}

pub(super) enum QuestionSource {
    ServerRequest(CodexRequestId),
    AsyncMessage(String),
}

#[derive(Deserialize)]
struct AsyncQuestion {
    title: String,
    #[serde(default)]
    options: Option<Vec<String>>,
}

pub(super) fn async_reply_display(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        question_item_id: String,
        question: String,
        answer: String,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Replies {
        Many(Vec<Reply>),
        One(Reply),
    }
    let body = text
        .trim()
        .strip_prefix("<send_user_message_question_reply>")?
        .strip_suffix("</send_user_message_question_reply>")?;
    let replies = match serde_json::from_str::<Replies>(body).ok()? {
        Replies::Many(replies) => replies,
        Replies::One(reply) => vec![reply],
    };
    if replies.is_empty() || replies.iter().any(|r| r.question_item_id.is_empty()) {
        return None;
    }
    Some(
        replies
            .into_iter()
            .map(|r| format!("> {}\n\n{}", r.question, r.answer))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    thread_id: String,
    turn_id: String,
    // Older app-server versions only sent blocking requests.
    #[serde(default = "blocking")]
    is_blocking: bool,
    questions: Vec<NativeQuestion>,
}

fn blocking() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeQuestion {
    id: String,
    header: String,
    question: String,
    options: Option<Vec<QuestionOption>>,
    #[serde(default)]
    is_other: bool,
    #[serde(default)]
    is_secret: bool,
}

pub(super) fn input_id(id: &CodexRequestId) -> String {
    match id {
        CodexRequestId::Number(value) => value.to_string(),
        CodexRequestId::String(value) => format!("string:{value}"),
    }
}

impl PendingQuestion {
    pub fn parse(id: CodexRequestId, params: Value) -> Result<Self, String> {
        let params: Params = serde_json::from_value(params)
            .map_err(|_| "Invalid Codex question request".to_owned())?;
        let mut ids = HashSet::new();
        if params.thread_id.is_empty()
            || params.turn_id.is_empty()
            || params.questions.is_empty()
            || params
                .questions
                .iter()
                .any(|q| q.id.is_empty() || q.question.trim().is_empty() || !ids.insert(&q.id))
        {
            return Err("Codex question request has missing or duplicate identifiers".into());
        }
        let request = QuestionRequest {
            id: input_id(&id),
            blocking: params.is_blocking,
            questions: params
                .questions
                .into_iter()
                .map(|q| {
                    let options = q.options.unwrap_or_default();
                    Question {
                        id: q.id,
                        header: q.header,
                        prompt: q.question,
                        allow_free_text: q.is_other || options.is_empty(),
                        options,
                        secret: q.is_secret,
                    }
                })
                .collect(),
        };
        Ok(Self {
            source: QuestionSource::ServerRequest(id),
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            request,
        })
    }

    pub fn parse_async(params: &Value) -> Result<Self, String> {
        let invalid = || "Invalid Codex async question message".to_owned();
        let item = &params["item"];
        let item_id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        let thread_id = params["threadId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        let turn_id = params["turnId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        let questions: Vec<AsyncQuestion> =
            serde_json::from_value(item["questions"].clone()).map_err(|_| invalid())?;
        if questions.is_empty()
            || questions.iter().any(|q| {
                q.title.trim().is_empty()
                    || q.options.as_ref().is_some_and(|o| {
                        o.is_empty() || o.iter().any(|label| label.trim().is_empty())
                    })
            })
        {
            return Err(invalid());
        }
        Ok(Self {
            source: QuestionSource::AsyncMessage(item_id.to_owned()),
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
            request: QuestionRequest {
                id: format!("async:{item_id}"),
                blocking: false,
                questions: questions
                    .into_iter()
                    .enumerate()
                    .map(|(index, question)| Question {
                        id: index.to_string(),
                        header: "Question".into(),
                        prompt: question.title,
                        options: question
                            .options
                            .unwrap_or_default()
                            .into_iter()
                            .map(|label| QuestionOption {
                                label,
                                description: String::new(),
                            })
                            .collect(),
                        allow_free_text: true,
                        secret: false,
                    })
                    .collect(),
            },
        })
    }

    pub fn async_reply(&self, response: &QuestionResponse) -> Result<String, String> {
        if response.cancelled {
            return Err("Cancelled async questions do not send a reply".into());
        }
        self.reply(response)?;
        let QuestionSource::AsyncMessage(item_id) = &self.source else {
            return Err("Question is not an async message".into());
        };
        let replies: Vec<_> = self
            .request
            .questions
            .iter()
            .map(|question| {
                json!({
                    "questionItemId": item_id,
                    "question": question.prompt,
                    "answer": response.answers[&question.id].join("\n"),
                })
            })
            .collect();
        Ok(format!(
            "<send_user_message_question_reply>{}</send_user_message_question_reply>",
            json!(replies)
        ))
    }

    pub fn reply(&self, response: &QuestionResponse) -> Result<Value, String> {
        if response.cancelled {
            return Ok(json!({"answers": {}}));
        }
        if response.answers.len() != self.request.questions.len() {
            return Err("Answer each question before submitting".into());
        }
        for question in &self.request.questions {
            let answers = response
                .answers
                .get(&question.id)
                .ok_or_else(|| "Answer each question before submitting".to_owned())?;
            if answers.is_empty()
                || answers.iter().any(|answer| {
                    answer.trim().is_empty()
                        || (!question.allow_free_text
                            && !question.options.iter().any(|o| &o.label == answer))
                })
            {
                return Err("Invalid question answer".into());
            }
        }
        let answers: BTreeMap<_, _> = response
            .answers
            .iter()
            .map(|(id, answers)| (id, json!({"answers": answers})))
            .collect();
        Ok(json!({"answers": answers}))
    }
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;
