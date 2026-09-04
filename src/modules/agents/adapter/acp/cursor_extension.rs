use serde_json::{Value, json};

use crate::agents::{WorkerActivity, WorkerEvent, WorkerInput};

use super::wire::AcpRequestId;

pub(super) enum CursorRequest {
    Questions {
        title: String,
        questions: Vec<CursorQuestion>,
    },
    Plan {
        prompt: String,
    },
}

pub(super) struct CursorQuestion {
    pub(super) id: String,
    pub(super) prompt: String,
    pub(super) options: Vec<(String, String)>,
    pub(super) allow_multiple: bool,
}

pub(super) fn request(method: &str, params: &Value) -> Option<CursorRequest> {
    match method {
        "cursor/ask_question" => {
            let questions = params
                .get("questions")?
                .as_array()?
                .iter()
                .filter_map(parse_question)
                .collect::<Vec<_>>();
            (!questions.is_empty()).then(|| CursorRequest::Questions {
                title: params
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Cursor needs input")
                    .to_owned(),
                questions,
            })
        }
        "cursor/create_plan" => {
            let plan = params.get("plan")?.as_str()?;
            let title = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Cursor proposes a plan");
            let overview = params
                .get("overview")
                .and_then(Value::as_str)
                .map(|overview| format!("{overview}\n\n"))
                .unwrap_or_default();
            Some(CursorRequest::Plan {
                prompt: format!("{title}\n\n{overview}{plan}"),
            })
        }
        _ => None,
    }
}

fn parse_question(value: &Value) -> Option<CursorQuestion> {
    let id = value.get("id")?.as_str()?.to_owned();
    let prompt = value.get("prompt")?.as_str()?.to_owned();
    let options = value
        .get("options")?
        .as_array()?
        .iter()
        .filter_map(|option| {
            Some((
                option.get("label")?.as_str()?.to_owned(),
                option.get("id")?.as_str()?.to_owned(),
            ))
        })
        .collect::<Vec<_>>();
    (!options.is_empty()).then_some(CursorQuestion {
        id,
        prompt,
        options,
        allow_multiple: value
            .get("allowMultiple")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(super) fn question_input(
    request: &AcpRequestId,
    title: &str,
    question: &CursorQuestion,
    index: usize,
    option_index: usize,
) -> WorkerInput {
    let (prompt, options) = if question.allow_multiple {
        let label = &question.options[option_index].0;
        (
            format!("{title}\n\n{}\n\nSelect “{label}”?", question.prompt),
            vec!["Include".into(), "Skip".into()],
        )
    } else {
        (
            format!("{title}\n\n{}", question.prompt),
            question
                .options
                .iter()
                .map(|(label, _)| label.clone())
                .collect(),
        )
    };
    WorkerInput {
        id: format!(
            "cursor-question:{}:{index}:{option_index}",
            request_id(request)
        ),
        prompt,
        options,
        secret: false,
    }
}

pub(super) fn plan_input(request: &AcpRequestId, prompt: String) -> WorkerInput {
    WorkerInput {
        id: format!("cursor-plan:{}", request_id(request)),
        prompt,
        options: vec!["Accept".into(), "Reject".into()],
        secret: false,
    }
}

pub(super) fn notification(method: &str, params: &Value) -> Option<(WorkerEvent, WorkerEvent)> {
    let (name, result) = match method {
        "cursor/task" => (
            "cursor_task",
            json!({
                "description": params.get("description"),
                "agentId": params.get("agentId"),
                "durationMs": params.get("durationMs"),
            }),
        ),
        "cursor/update_todos" => (
            "cursor_todos",
            json!({"todos": params.get("todos"), "merge": params.get("merge")}),
        ),
        "cursor/generate_image" => (
            "cursor_generate_image",
            json!({
                "description": params.get("description"),
                "filePath": params.get("filePath"),
            }),
        ),
        _ => return None,
    };
    let id = params
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or(method)
        .to_owned();
    Some((
        WorkerEvent::Activity(WorkerActivity::ToolStarted {
            id: id.clone(),
            name: name.into(),
            args: params.clone(),
        }),
        WorkerEvent::Activity(WorkerActivity::ToolFinished {
            id,
            result: json!([{"type": "text", "text": result.to_string()}]),
            is_error: false,
        }),
    ))
}

fn request_id(id: &AcpRequestId) -> String {
    match id {
        AcpRequestId::Number(value) => value.to_string(),
        AcpRequestId::String(value) => value.clone(),
        AcpRequestId::Null => "null".into(),
    }
}
