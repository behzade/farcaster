use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashSet;

use super::contract::{
    DataEnvelope, ErrorEnvelope, OpenCodeDelivery, OpenCodeFileInput, OpenCodeHttpMethod,
    OpenCodeHttpRequest, OpenCodeHttpResponse, OpenCodeHttpTransport, OpenCodeLocation,
    OpenCodePromptAdmission, OpenCodePromptDispatchError, OpenCodeSession,
};

pub struct OpenCodeClient<T> {
    transport: T,
}

impl<T: OpenCodeHttpTransport> OpenCodeClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn create_session(
        &mut self,
        directory: &str,
        parent_id: Option<&str>,
        model: Option<(&str, &str, Option<&str>)>,
    ) -> Result<OpenCodeSession, String> {
        self.json(
            OpenCodeHttpMethod::Post,
            "/api/session".into(),
            Some(json!({
                "location": {"directory": directory},
                "parentID": parent_id,
                "model": model.map(|(provider_id, model_id, variant)| {
                    model_selection(provider_id, "id", model_id, variant)
                }),
            })),
        )
    }

    pub fn fork_session(
        &mut self,
        session_id: &str,
        model: Option<(&str, &str, Option<&str>)>,
    ) -> Result<OpenCodeSession, String> {
        self.json(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/fork", path_segment(session_id)),
            Some(json!({
                "model": model.map(|(provider_id, model_id, variant)| {
                    model_selection(provider_id, "id", model_id, variant)
                }),
            })),
        )
    }

    pub fn get_session(&mut self, session_id: &str) -> Result<OpenCodeSession, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session/{}", path_segment(session_id)),
            None,
        )
    }

    pub fn prompt(
        &mut self,
        session_id: &str,
        id: Option<&str>,
        submission_id: Option<&str>,
        text: &str,
        files: Vec<OpenCodeFileInput>,
        delivery: OpenCodeDelivery,
    ) -> Result<OpenCodePromptAdmission, OpenCodePromptDispatchError> {
        let path = format!("/api/session/{}/prompt", path_segment(session_id));
        let mut body = json!({
            "id": id,
            "text": text,
            "files": files,
            "agents": [],
            "delivery": delivery,
            "resume": true,
        });
        if let Some(submission_id) = submission_id {
            body["metadata"] = json!({"farcasterSubmissionId": submission_id});
        }
        let response = self.dispatch_input(path, body)?;
        decode_data(response).map_err(OpenCodePromptDispatchError::Unknown)
    }

    pub fn run_command(
        &mut self,
        session_id: &str,
        command: &str,
        text: &str,
        files: Vec<OpenCodeFileInput>,
    ) -> Result<(), OpenCodePromptDispatchError> {
        let (operation, body) = if command == "compact" {
            ("compact", json!({}))
        } else {
            (
                "command",
                json!({"command": command, "text": text, "files": files, "agents": [], "delivery": "queue"}),
            )
        };
        self.dispatch_input(
            format!("/api/session/{}/{operation}", path_segment(session_id)),
            body,
        )?;
        Ok(())
    }

    fn dispatch_input(
        &mut self,
        path: String,
        body: Value,
    ) -> Result<OpenCodeHttpResponse, OpenCodePromptDispatchError> {
        let body = serde_json::to_vec(&body)
            .map_err(|error| OpenCodePromptDispatchError::Unsent(error.to_string()))?;
        let response = self.transport.execute_prompt(OpenCodeHttpRequest {
            method: OpenCodeHttpMethod::Post,
            path,
            body: Some(body),
        })?;
        if !(200..300).contains(&response.status) {
            let error = ensure_success(&response).expect_err("non-success response");
            return Err(if (400..500).contains(&response.status) {
                OpenCodePromptDispatchError::Unsent(error)
            } else {
                OpenCodePromptDispatchError::Unknown(error)
            });
        }
        Ok(response)
    }

    pub fn context(&mut self, session_id: &str) -> Result<Vec<Value>, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session/{}/context", path_segment(session_id)),
            None,
        )
    }

    pub fn interrupt(&mut self, session_id: &str, resume: bool) -> Result<bool, String> {
        #[derive(serde::Deserialize)]
        struct InterruptReceipt {
            interrupted: bool,
        }

        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!(
                "/api/session/{}/interrupt?continue={resume}",
                path_segment(session_id)
            ),
            None,
        )?;
        ensure_success(&response)?;
        // Interrupt returns a receipt directly, without a data envelope.
        serde_json::from_slice::<InterruptReceipt>(&response.body)
            .map(|receipt| receipt.interrupted)
            .map_err(|error| format!("decode OpenCode interrupt response: {error}"))
    }

    pub fn models(&mut self, directory: &str) -> Result<Value, String> {
        let directory =
            url::form_urlencoded::byte_serialize(directory.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/model?directory={directory}"),
            None,
        )
    }

    pub fn agents(&mut self, directory: &str) -> Result<Value, String> {
        let directory =
            url::form_urlencoded::byte_serialize(directory.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/agent?directory={directory}"),
            None,
        )
    }

    pub fn default_model(
        &mut self,
        directory: &str,
    ) -> Result<Option<super::contract::OpenCodeModelSelection>, String> {
        let directory =
            url::form_urlencoded::byte_serialize(directory.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/model/default?directory={directory}"),
            None,
        )
    }

    pub fn commands(&mut self, directory: &str) -> Result<Value, String> {
        let directory =
            url::form_urlencoded::byte_serialize(directory.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/command?directory={directory}"),
            None,
        )
    }

    pub fn list_sessions(&mut self, query: &str) -> Result<Value, String> {
        let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session?limit=100&order=desc&search={encoded}"),
            None,
        )
    }

    pub fn session_messages(&mut self, session_id: &str) -> Result<Value, String> {
        const PAGE_LIMIT: u32 = 200;
        let session_id = path_segment(session_id);
        let mut messages = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        loop {
            let path = match &cursor {
                None => format!("/api/session/{session_id}/message?limit={PAGE_LIMIT}&order=asc"),
                Some(cursor) => {
                    let encoded: String =
                        url::form_urlencoded::byte_serialize(cursor.as_bytes()).collect();
                    format!("/api/session/{session_id}/message?limit={PAGE_LIMIT}&cursor={encoded}")
                }
            };
            let operation = format!("{:?} {path}", OpenCodeHttpMethod::Get);
            let response = self.execute(OpenCodeHttpMethod::Get, path, None)?;
            ensure_success(&response)?;
            let page: OpenCodeMessagePage = serde_json::from_slice(&response.body)
                .map_err(|error| format!("{operation}: decode OpenCode response: {error}"))?;
            for message in page.data {
                if let Some(id) = message.get("id").and_then(Value::as_str)
                    && !seen_ids.insert(id.to_owned())
                {
                    continue;
                }
                messages.push(message);
            }
            let next = page.cursor.next.filter(|cursor| !cursor.is_empty());
            match next {
                Some(next) if !seen_cursors.insert(next.clone()) => {
                    return Err("OpenCode session history cursor repeated".into());
                }
                Some(_) if seen_cursors.len() > 256 => {
                    return Err("OpenCode session history exceeded 256 pages".into());
                }
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(Value::Array(messages))
    }

    pub fn reply_permission(
        &mut self,
        session_id: &str,
        request_id: &str,
        reply: &str,
    ) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!(
                "/api/session/{}/permission/{}/reply",
                path_segment(session_id),
                path_segment(request_id)
            ),
            Some(json!({"decision": reply})),
        )?;
        ensure_success(&response)
    }

    pub fn reply_form(
        &mut self,
        session_id: &str,
        form_id: &str,
        answer: Value,
    ) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!(
                "/api/session/{}/form/{}/reply",
                path_segment(session_id),
                path_segment(form_id)
            ),
            Some(json!({"answer": answer})),
        )?;
        ensure_success(&response)
    }

    pub fn cancel_form(&mut self, session_id: &str, form_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!(
                "/api/session/{}/form/{}/cancel",
                path_segment(session_id),
                path_segment(form_id)
            ),
            None,
        )?;
        ensure_success(&response)
    }

    pub fn select_agent(&mut self, session_id: &str, agent: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/agent", path_segment(session_id)),
            Some(json!({"agent": agent})),
        )?;
        ensure_success(&response)
    }

    pub fn select_model(
        &mut self,
        session_id: &str,
        provider: &str,
        model: &str,
        variant: Option<&str>,
    ) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/model", path_segment(session_id)),
            Some(json!({
                "model": model_selection(provider, "id", model, variant)
            })),
        )?;
        ensure_success(&response)
    }

    pub fn compact_session(&mut self, session_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/compact", path_segment(session_id)),
            Some(json!({})),
        )?;
        ensure_success(&response)
    }

    pub fn rename_session(&mut self, session_id: &str, title: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/rename", path_segment(session_id)),
            Some(json!({"title": title})),
        )?;
        ensure_success(&response)
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Delete,
            format!("/api/session/{}", path_segment(session_id)),
            None,
        )?;
        decode_empty(response)
    }

    pub fn move_session(
        &mut self,
        session_id: &str,
        location: &OpenCodeLocation,
    ) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/move", path_segment(session_id)),
            Some(json!(location)),
        )?;
        decode_empty(response)
    }

    pub fn session_inbox(&mut self, session_id: &str) -> Result<Vec<Value>, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session/{}/inbox", path_segment(session_id)),
            None,
        )
    }

    pub fn cancel_inbox(&mut self, session_id: &str, inbox_id: &str) -> Result<bool, String> {
        let response = self.execute(
            OpenCodeHttpMethod::Delete,
            format!(
                "/api/session/{}/inbox/{}",
                path_segment(session_id),
                path_segment(inbox_id)
            ),
            None,
        )?;
        if response.status == 409 {
            return Ok(false);
        }
        ensure_success(&response)?;
        Ok(true)
    }

    pub fn steer_inbox(&mut self, session_id: &str, inbox_id: &str) -> Result<bool, String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!(
                "/api/session/{}/inbox/{}/steer",
                path_segment(session_id),
                path_segment(inbox_id)
            ),
            None,
        )?;
        if response.status == 409 {
            return Ok(false);
        }
        ensure_success(&response)?;
        Ok(true)
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn json<R: DeserializeOwned>(
        &mut self,
        method: OpenCodeHttpMethod,
        path: String,
        body: Option<Value>,
    ) -> Result<R, String> {
        let operation = format!("{method:?} {path}");
        let response = self.execute(method, path, body)?;
        ensure_success(&response)?;
        decode_data(response).map_err(|error| format!("{operation}: {error}"))
    }

    fn execute(
        &mut self,
        method: OpenCodeHttpMethod,
        path: String,
        body: Option<Value>,
    ) -> Result<OpenCodeHttpResponse, String> {
        let body = body
            .map(|body| serde_json::to_vec(&body).map_err(|error| error.to_string()))
            .transpose()?;
        self.transport
            .execute(OpenCodeHttpRequest { method, path, body })
    }
}

#[derive(Debug, Deserialize)]
struct OpenCodeMessagePage {
    data: Vec<Value>,
    #[serde(default)]
    cursor: OpenCodePageCursor,
}

#[derive(Debug, Default, Deserialize)]
struct OpenCodePageCursor {
    #[serde(default)]
    next: Option<String>,
}

fn model_selection(provider: &str, id_key: &str, model: &str, variant: Option<&str>) -> Value {
    let mut selection = serde_json::Map::from_iter([
        ("providerID".into(), Value::String(provider.into())),
        (id_key.into(), Value::String(model.into())),
    ]);
    if let Some(variant) = variant {
        selection.insert("variant".into(), Value::String(variant.into()));
    }
    Value::Object(selection)
}

fn decode_data<T: DeserializeOwned>(response: OpenCodeHttpResponse) -> Result<T, String> {
    ensure_success(&response)?;
    serde_json::from_slice::<DataEnvelope<T>>(&response.body)
        .map(|response| response.data)
        .map_err(|error| format!("decode OpenCode response: {error}"))
}

fn decode_empty(response: OpenCodeHttpResponse) -> Result<(), String> {
    ensure_success(&response)
}

fn ensure_success(response: &OpenCodeHttpResponse) -> Result<(), String> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    match serde_json::from_slice::<ErrorEnvelope>(&response.body) {
        Ok(error) => Err(format!(
            "OpenCode API error {} ({}): {}",
            response.status, error.tag, error.message
        )),
        Err(_) => Err(format!("OpenCode API returned HTTP {}", response.status)),
    }
}

fn path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
