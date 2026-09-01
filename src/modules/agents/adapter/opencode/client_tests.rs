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
        response(204, Value::Null),
        response(204, Value::Null),
    ]);
    let mut client = OpenCodeClient::new(transport);

    let created = client.create_session("/project", Some("parent-1"), None)?;
    assert_eq!(created.parent_id.as_deref(), Some("parent-1"));
    assert_eq!(client.get_session("session/1")?.id, "session/1");
    let admission = client.prompt(
        "session/1",
        "inspect this",
        vec![OpenCodeFileInput {
            uri: "file:///tmp/image.png".into(),
            name: Some("image.png".into()),
            description: None,
        }],
        OpenCodeDelivery::Queue,
    )?;
    assert_eq!(admission.session_id, "session/1");
    client.interrupt("session/1")?;
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
            "text": "inspect this",
            "files": [{"uri": "file:///tmp/image.png", "name": "image.png"}],
            "agents": [],
            "delivery": "queue",
            "resume": true
        })
    );
    assert_eq!(body(&transport.requests[3]), json!({"continue": false}));
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

    client.prompt("session-1", "more", Vec::new(), OpenCodeDelivery::Steer)?;

    assert_eq!(
        body(&client.into_transport().requests[0])["delivery"],
        "steer"
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
