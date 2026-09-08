use std::io::Cursor;

use serde_json::json;

use super::*;

const PROFILE: AcpProfile = AcpProfile {
    backend: "example-acp",
    name: "Example ACP",
    command: "example",
    path_environment: "EXAMPLE_ACP_PATH",
    arguments: &["acp"],
    auth_method: Some("login"),
    force_argument: None,
};

#[test]
fn initialize_authenticates_with_an_advertised_method() -> Result<(), String> {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":1,\"authMethods\":[{\"id\":\"login\"}]}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":null}\n",
    );
    let mut connection = AcpConnection::new(Cursor::new(input.as_bytes()), Vec::new());

    assert_eq!(connection.initialize(&PROFILE)?["protocolVersion"], 1);
    let (_, output, _, _) = connection.into_parts();
    let output = String::from_utf8(output).map_err(|error| error.to_string())?;
    assert!(output.contains("\"method\":\"initialize\""));
    assert!(output.contains("\"method\":\"authenticate\""));
    Ok(())
}

#[test]
fn wait_response_preserves_interleaved_updates() -> Result<(), String> {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"sessionId\":\"one\"}}\n",
    );
    let mut connection = AcpConnection::new(Cursor::new(input.as_bytes()), Vec::new());
    let id = connection.send_request("session/new", json!({}))?;

    assert_eq!(connection.wait_response(&id)?["sessionId"], "one");
    assert_eq!(connection.drain_queued().len(), 1);
    Ok(())
}
