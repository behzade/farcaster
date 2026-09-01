use std::{
    collections::VecDeque,
    io::{BufRead, Write},
};

use serde_json::{Value, json};

use super::{
    AcpProfile,
    wire::{AcpInbound, AcpRequestId, decode_frame, encode_request},
};

pub(super) struct AcpConnection<R, W> {
    reader: R,
    writer: W,
    queued: VecDeque<AcpInbound>,
    next_id: i64,
}

impl<R: BufRead, W: Write> AcpConnection<R, W> {
    pub(super) fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            queued: VecDeque::new(),
            next_id: 0,
        }
    }

    pub(super) fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<AcpRequestId, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "ACP request id overflow".to_owned())?;
        let id = AcpRequestId::Number(self.next_id);
        self.writer
            .write_all(&encode_request(&id, method, params)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write ACP request: {error}"))?;
        Ok(id)
    }

    pub(super) fn initialize(&mut self, profile: &AcpProfile) -> Result<Value, String> {
        let id = self.send_request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                },
                "clientInfo": {
                    "name": "farcaster",
                    "title": "Farcaster",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;
        let initialized = self.wait_response(&id)?;
        if let Some(method_id) = profile.auth_method {
            let advertised = initialized
                .get("authMethods")
                .and_then(Value::as_array)
                .is_some_and(|methods| {
                    methods
                        .iter()
                        .any(|method| method.get("id").and_then(Value::as_str) == Some(method_id))
                });
            if !advertised {
                return Err(format!(
                    "{} ACP agent did not advertise authentication method {method_id}",
                    profile.name
                ));
            }
            let id = self.send_request("authenticate", json!({"methodId": method_id}))?;
            self.wait_response(&id)?;
        }
        Ok(initialized)
    }

    pub(super) fn wait_response(&mut self, expected: &AcpRequestId) -> Result<Value, String> {
        if let Some(index) = self
            .queued
            .iter()
            .position(|message| matches_response(message, expected))
        {
            let message = self
                .queued
                .remove(index)
                .expect("queued ACP response exists");
            return decode_response(message);
        }
        loop {
            let message = read_message(&mut self.reader)?;
            if matches_response(&message, expected) {
                return decode_response(message);
            }
            self.queued.push_back(message);
        }
    }

    pub(super) fn drain_queued(&mut self) -> VecDeque<AcpInbound> {
        self.queued.drain(..).collect()
    }

    pub(super) fn into_parts(self) -> (R, W, VecDeque<AcpInbound>, i64) {
        (self.reader, self.writer, self.queued, self.next_id)
    }
}

pub(super) fn read_message(reader: &mut impl BufRead) -> Result<AcpInbound, String> {
    let mut frame = Vec::new();
    let read = reader
        .read_until(b'\n', &mut frame)
        .map_err(|error| format!("read ACP response: {error}"))?;
    if read == 0 {
        return Err("ACP agent closed its output".into());
    }
    while matches!(frame.last(), Some(b'\n' | b'\r')) {
        frame.pop();
    }
    decode_frame(&frame)
}

fn matches_response(message: &AcpInbound, expected: &AcpRequestId) -> bool {
    matches!(
        message,
        AcpInbound::Response { id, .. } | AcpInbound::Error { id, .. } if id == expected
    )
}

fn decode_response(message: AcpInbound) -> Result<Value, String> {
    match message {
        AcpInbound::Response { result, .. } => Ok(result),
        AcpInbound::Error { code, message, .. } => Err(format!("ACP error {code}: {message}")),
        _ => Err("ACP message is not a response".into()),
    }
}

#[cfg(test)]
mod tests {
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
}
