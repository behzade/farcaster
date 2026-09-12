use std::collections::VecDeque;

use serde_json::{Value, json};

use super::{
    client::OpenCodeClient,
    contract::{
        OpenCodeDelivery, OpenCodeFileInput, OpenCodeHttpMethod, OpenCodeHttpRequest,
        OpenCodeHttpResponse, OpenCodeHttpTransport,
    },
};

#[derive(Default)]
struct FakeTransport {
    responses: VecDeque<OpenCodeHttpResponse>,
    requests: Vec<OpenCodeHttpRequest>,
}

impl FakeTransport {
    fn with_responses(responses: impl IntoIterator<Item = OpenCodeHttpResponse>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            requests: Vec::new(),
        }
    }
}

impl OpenCodeHttpTransport for FakeTransport {
    fn execute(&mut self, request: OpenCodeHttpRequest) -> Result<OpenCodeHttpResponse, String> {
        self.requests.push(request);
        self.responses
            .pop_front()
            .ok_or_else(|| "missing fake OpenCode response".into())
    }
}

fn response(status: u16, body: Value) -> OpenCodeHttpResponse {
    OpenCodeHttpResponse {
        status,
        body: serde_json::to_vec(&body).expect("test JSON serializes"),
    }
}

fn body(request: &OpenCodeHttpRequest) -> Value {
    serde_json::from_slice(request.body.as_ref().expect("request has body"))
        .expect("request body is JSON")
}

fn prompt_error(error: super::contract::OpenCodePromptDispatchError) -> String {
    match error {
        super::contract::OpenCodePromptDispatchError::Unsent(error)
        | super::contract::OpenCodePromptDispatchError::Unknown(error) => error,
    }
}

#[test]
fn native_vertical_slice_preserves_session_and_prompt_features() -> Result<(), String> {
    let session = json!({
        "data": {
            "id": "session/1",
            "location": {"directory": "/project"},
            "parentID": "parent-1",
            "title": "Work"
        }
    });
    let transport = FakeTransport::with_responses([
        response(200, session.clone()),
        response(200, session),
        response(
            200,
            json!({"data": {"id": "prompt-1", "sessionID": "session/1", "delivery": "queue"}}),
        ),
        response(200, json!({"interrupted": true})),
        response(204, Value::Null),
    ]);
    let mut client = OpenCodeClient::new(transport);

    let created = client.create_session("/project", Some("parent-1"), None)?;
    assert_eq!(created.parent_id.as_deref(), Some("parent-1"));
    assert_eq!(client.get_session("session/1")?.id, "session/1");
    let admission = client
        .prompt(
            "session/1",
            Some("msg_prompt-1"),
            "inspect this",
            vec![OpenCodeFileInput {
                uri: "file:///tmp/image.png".into(),
                name: Some("image.png".into()),
                description: None,
            }],
            OpenCodeDelivery::Queue,
        )
        .map_err(prompt_error)?;
    assert_eq!(admission.session_id, "session/1");
    assert!(client.interrupt("session/1", false)?);
    client.delete_session("session/1")?;

    let transport = client.into_transport();
    assert_eq!(transport.requests[0].path, "/api/session");
    assert_eq!(
        body(&transport.requests[0]),
        json!({"location": {"directory": "/project"}, "parentID": "parent-1", "model": null})
    );
    assert_eq!(transport.requests[1].path, "/api/session/session%2F1");
    assert_eq!(
        body(&transport.requests[2]),
        json!({
            "id": "msg_prompt-1",
            "text": "inspect this",
            "files": [{"uri": "file:///tmp/image.png", "name": "image.png"}],
            "agents": [],
            "delivery": "queue",
            "resume": true
        })
    );
    assert_eq!(
        transport.requests[3].path,
        "/api/session/session%2F1/interrupt?continue=false"
    );
    assert!(transport.requests[3].body.is_none());
    assert_eq!(transport.requests[4].method, OpenCodeHttpMethod::Delete);
    Ok(())
}

#[test]
fn steer_is_encoded_independently_from_queue() -> Result<(), String> {
    let transport = FakeTransport::with_responses([response(
        200,
        json!({"data": {"id": "prompt-1", "sessionID": "session-1", "delivery": "steer"}}),
    )]);
    let mut client = OpenCodeClient::new(transport);

    client
        .prompt(
            "session-1",
            Some("msg_prompt-1"),
            "more",
            Vec::new(),
            OpenCodeDelivery::Steer,
        )
        .map_err(prompt_error)?;

    assert_eq!(
        body(&client.into_transport().requests[0])["delivery"],
        "steer"
    );
    Ok(())
}

#[test]
fn prompt_only_rejects_receipts_that_prove_no_admission() {
    for (response, unknown) in [
        (
            response(409, json!({"_tag":"Conflict","message":"busy"})),
            false,
        ),
        (
            response(500, json!({"_tag":"Internal","message":"failed"})),
            true,
        ),
        (
            response(200, json!({"data": {"id": "missing-fields"}})),
            true,
        ),
    ] {
        let mut client = OpenCodeClient::new(FakeTransport::with_responses([response]));
        let error = client
            .prompt(
                "session-1",
                Some("msg_prompt-1"),
                "work",
                Vec::new(),
                OpenCodeDelivery::Queue,
            )
            .expect_err("prompt should fail");
        assert_eq!(
            matches!(
                error,
                super::contract::OpenCodePromptDispatchError::Unknown(_)
            ),
            unknown
        );
    }
}

#[test]
fn apply_steering_uses_continue_query_and_reports_idle() -> Result<(), String> {
    for interrupted in [true, false] {
        let transport =
            FakeTransport::with_responses([response(200, json!({"interrupted": interrupted}))]);
        let mut client = OpenCodeClient::new(transport);
        assert_eq!(client.interrupt("session/1", true)?, interrupted);
        let requests = client.into_transport().requests;
        assert_eq!(requests[0].method, OpenCodeHttpMethod::Post);
        assert_eq!(
            requests[0].path,
            "/api/session/session%2F1/interrupt?continue=true"
        );
        assert!(requests[0].body.is_none());
    }
    Ok(())
}

#[test]
fn model_selection_omits_an_unset_variant() -> Result<(), String> {
    let session = json!({
        "data": {
            "id": "session-1",
            "location": {"directory": "/project"}
        }
    });
    let transport = FakeTransport::with_responses([
        response(200, session),
        response(204, Value::Null),
        response(204, Value::Null),
    ]);
    let mut client = OpenCodeClient::new(transport);

    client.create_session("/project", None, Some(("openai", "gpt-5", None)))?;
    client.select_model("session-1", "openai", "gpt-5", None)?;
    client.select_model("session-1", "openai", "gpt-5", Some("high"))?;

    let requests = client.into_transport().requests;
    assert_eq!(
        body(&requests[0])["model"],
        json!({"providerID": "openai", "id": "gpt-5"})
    );
    assert_eq!(
        body(&requests[1])["model"],
        json!({"providerID": "openai", "id": "gpt-5"})
    );
    assert_eq!(
        body(&requests[2])["model"],
        json!({"providerID": "openai", "id": "gpt-5", "variant": "high"})
    );
    Ok(())
}

#[test]
fn permission_replies_use_requested_session() -> Result<(), String> {
    let transport = FakeTransport::with_responses([response(204, Value::Null)]);
    let mut client = OpenCodeClient::new(transport);

    client.reply_permission("child/1", "permission/1", "once")?;

    let transport = client.into_transport();
    let request = &transport.requests[0];
    assert_eq!(request.method, OpenCodeHttpMethod::Post);
    assert_eq!(
        request.path,
        "/api/session/child%2F1/permission/permission%2F1/reply"
    );
    assert_eq!(body(request), json!({"reply": "once"}));
    Ok(())
}

#[test]
fn api_errors_preserve_status_tag_and_message() {
    let transport = FakeTransport::with_responses([response(
        409,
        json!({"_tag": "SessionBusy", "message": "already running"}),
    )]);
    let mut client = OpenCodeClient::new(transport);

    let error = client
        .get_session("session-1")
        .expect_err("request should fail");
    assert_eq!(
        error,
        "OpenCode API error 409 (SessionBusy): already running"
    );
}
