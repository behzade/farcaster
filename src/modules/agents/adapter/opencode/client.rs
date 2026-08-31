use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::contract::{
    DataEnvelope, ErrorEnvelope, OpenCodeDelivery, OpenCodeFileInput, OpenCodeHttpMethod,
    OpenCodeHttpRequest, OpenCodeHttpResponse, OpenCodeHttpTransport, OpenCodePromptAdmission,
    OpenCodeSession,
};

pub(crate) struct OpenCodeClient<T> {
    transport: T,
}

impl<T: OpenCodeHttpTransport> OpenCodeClient<T> {
    pub(crate) fn new(transport: T) -> Self {
        Self { transport }
    }

    pub(crate) fn create_session(
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
                "model": model.map(|(provider_id, model_id, variant)| json!({
                    "providerID": provider_id,
                    "modelID": model_id,
                    "variant": variant,
                })),
            })),
        )
    }

    pub(crate) fn fork_session(
        &mut self,
        session_id: &str,
        model: Option<(&str, &str, Option<&str>)>,
    ) -> Result<OpenCodeSession, String> {
        self.json(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/fork", path_segment(session_id)),
            Some(json!({
                "model": model.map(|(provider_id, model_id, variant)| json!({
                    "providerID": provider_id,
                    "modelID": model_id,
                    "variant": variant,
                })),
            })),
        )
    }

    pub(crate) fn get_session(&mut self, session_id: &str) -> Result<OpenCodeSession, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session/{}", path_segment(session_id)),
            None,
        )
    }

    pub(crate) fn prompt(
        &mut self,
        session_id: &str,
        text: &str,
        files: Vec<OpenCodeFileInput>,
        delivery: OpenCodeDelivery,
    ) -> Result<OpenCodePromptAdmission, String> {
        self.json(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/prompt", path_segment(session_id)),
            Some(json!({
                "prompt": {"text": text, "files": files, "agents": []},
                "delivery": delivery,
                "resume": true,
            })),
        )
    }

    pub(crate) fn wait_session(&mut self, session_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/wait", path_segment(session_id)),
            None,
        )?;
        decode_empty(response)
    }

    pub(crate) fn context(&mut self, session_id: &str) -> Result<Vec<Value>, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session/{}/context", path_segment(session_id)),
            None,
        )
    }

    pub(crate) fn interrupt(&mut self, session_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Post,
            format!("/api/session/{}/interrupt", path_segment(session_id)),
            Some(json!({"continue": false})),
        )?;
        decode_empty(response)
    }

    pub(crate) fn list_sessions(&mut self, query: &str) -> Result<Value, String> {
        let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
        self.json(
            OpenCodeHttpMethod::Get,
            format!("/api/session?limit=100&order=desc&search={encoded}"),
            None,
        )
    }

    pub(crate) fn session_messages(&mut self, session_id: &str) -> Result<Value, String> {
        self.json(
            OpenCodeHttpMethod::Get,
            format!(
                "/api/session/{}/message?limit=500&order=asc",
                path_segment(session_id)
            ),
            None,
        )
    }

    pub(crate) fn delete_session(&mut self, session_id: &str) -> Result<(), String> {
        let response = self.execute(
            OpenCodeHttpMethod::Delete,
            format!("/api/session/{}", path_segment(session_id)),
            None,
        )?;
        decode_empty(response)
    }

    pub(crate) fn into_transport(self) -> T {
        self.transport
    }

    fn json<R: DeserializeOwned>(
        &mut self,
        method: OpenCodeHttpMethod,
        path: String,
        body: Option<Value>,
    ) -> Result<R, String> {
        let response = self.execute(method, path, body)?;
        decode_data(response)
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
