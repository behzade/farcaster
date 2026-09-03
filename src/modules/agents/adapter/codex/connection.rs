use std::{
    collections::VecDeque,
    io::{BufRead, Write},
};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    approvals_reviewer,
    contract::{
        CodexClientInfo, CodexInbound, CodexInitializeResponse, CodexRequestId, CodexThread,
        CodexTurn, CodexUserInput, ThreadResponse, TurnResponse,
    },
    wire::{decode_frame, encode_notification, encode_request, encode_response},
};

pub(crate) struct CodexConnection<R, W> {
    reader: R,
    writer: W,
    queued: VecDeque<CodexInbound>,
    next_id: i64,
}

impl<R: BufRead, W: Write> CodexConnection<R, W> {
    pub(crate) fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            queued: VecDeque::new(),
            next_id: 0,
        }
    }

    pub(crate) fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<CodexRequestId, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "Codex request id overflow".to_owned())?;
        let id = CodexRequestId::Number(self.next_id);
        self.writer
            .write_all(&encode_request(&id, method, params)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write Codex app-server request: {error}"))?;
        Ok(id)
    }

    pub(crate) fn send_notification(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), String> {
        self.writer
            .write_all(&encode_notification(method, params)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write Codex app-server notification: {error}"))
    }

    pub(crate) fn respond(&mut self, id: &CodexRequestId, result: Value) -> Result<(), String> {
        self.writer
            .write_all(&encode_response(id, result)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write Codex app-server response: {error}"))
    }

    pub(crate) fn next(&mut self) -> Result<CodexInbound, String> {
        if let Some(message) = self.queued.pop_front() {
            return Ok(message);
        }
        read_message(&mut self.reader)
    }

    pub(crate) fn wait_response<T: DeserializeOwned>(
        &mut self,
        id: &CodexRequestId,
    ) -> Result<T, String> {
        if let Some(index) = self
            .queued
            .iter()
            .position(|message| matches_response(message, id))
        {
            let message = self.queued.remove(index).expect("queued response exists");
            return decode_response(message);
        }
        loop {
            let message = read_message(&mut self.reader)?;
            if matches_response(&message, id) {
                return decode_response(message);
            }
            self.queued.push_back(message);
        }
    }

    pub(crate) fn initialize(
        &mut self,
        client: CodexClientInfo,
    ) -> Result<CodexInitializeResponse, String> {
        self.initialize_with_capabilities(client, Value::Null)
    }

    pub(crate) fn initialize_experimental(
        &mut self,
        client: CodexClientInfo,
    ) -> Result<CodexInitializeResponse, String> {
        self.initialize_with_capabilities(client, json!({"experimentalApi": true}))
    }

    fn initialize_with_capabilities(
        &mut self,
        client: CodexClientInfo,
        capabilities: Value,
    ) -> Result<CodexInitializeResponse, String> {
        let id = self.send_request(
            "initialize",
            json!({"clientInfo": client, "capabilities": capabilities}),
        )?;
        let response = self.wait_response(&id)?;
        self.send_notification("initialized", None)?;
        Ok(response)
    }

    pub(crate) fn start_thread(
        &mut self,
        cwd: &str,
        provider: Option<&str>,
        model: Option<&str>,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> Result<CodexThread, String> {
        self.start_thread_with_persistence(cwd, provider, model, access_mode, true)
    }

    pub(crate) fn start_ephemeral_thread(
        &mut self,
        cwd: &str,
        provider: Option<&str>,
        model: Option<&str>,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> Result<CodexThread, String> {
        self.start_thread_with_persistence(cwd, provider, model, access_mode, false)
    }

    fn start_thread_with_persistence(
        &mut self,
        cwd: &str,
        provider: Option<&str>,
        model: Option<&str>,
        access_mode: crate::agents::HarnessAccessMode,
        persist: bool,
    ) -> Result<CodexThread, String> {
        let id = self.send_request(
            "thread/start",
            json!({
                "cwd": cwd,
                "modelProvider": provider,
                "model": model,
                "approvalsReviewer": approvals_reviewer(access_mode),
                "ephemeral": !persist,
            }),
        )?;
        self.wait_response::<ThreadResponse>(&id)
            .map(|response| response.thread)
    }

    pub(crate) fn fork_thread(
        &mut self,
        thread_id: &str,
        cwd: &str,
        provider: Option<&str>,
        model: Option<&str>,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> Result<CodexThread, String> {
        let id = self.send_request(
            "thread/fork",
            json!({
                "threadId": thread_id,
                "cwd": cwd,
                "modelProvider": provider,
                "model": model,
                "approvalsReviewer": approvals_reviewer(access_mode),
            }),
        )?;
        self.wait_response::<ThreadResponse>(&id)
            .map(|response| response.thread)
    }

    pub(crate) fn resume_thread(
        &mut self,
        thread_id: &str,
        access_mode: crate::agents::HarnessAccessMode,
    ) -> Result<CodexThread, String> {
        let id = self.send_request(
            "thread/resume",
            json!({
                "threadId": thread_id,
                "approvalsReviewer": approvals_reviewer(access_mode),
            }),
        )?;
        self.wait_response::<ThreadResponse>(&id)
            .map(|response| response.thread)
    }

    pub(crate) fn start_turn(
        &mut self,
        thread_id: &str,
        input: Vec<CodexUserInput>,
    ) -> Result<CodexTurn, String> {
        let id = self.send_request("turn/start", json!({"threadId": thread_id, "input": input}))?;
        self.wait_response::<TurnResponse>(&id)
            .map(|response| response.turn)
    }

    pub(crate) fn interrupt_turn(&mut self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        let id = self.send_request(
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
        )?;
        self.wait_response::<Value>(&id).map(|_| ())
    }

    pub(crate) fn into_parts(self) -> (R, W, VecDeque<CodexInbound>, i64) {
        (self.reader, self.writer, self.queued, self.next_id)
    }

    #[cfg(test)]
    pub(crate) fn into_writer(self) -> W {
        self.writer
    }
}

pub(super) fn read_message(reader: &mut impl BufRead) -> Result<CodexInbound, String> {
    let mut frame = Vec::new();
    let read = reader
        .read_until(b'\n', &mut frame)
        .map_err(|error| format!("read Codex app-server response: {error}"))?;
    if read == 0 {
        return Err("Codex app-server closed its output".into());
    }
    if frame.last() == Some(&b'\n') {
        frame.pop();
        if frame.last() == Some(&b'\r') {
            frame.pop();
        }
    }
    decode_frame(&frame)
}

fn matches_response(message: &CodexInbound, expected: &CodexRequestId) -> bool {
    matches!(
        message,
        CodexInbound::Response { id, .. } | CodexInbound::Error { id, .. } if id == expected
    )
}

fn decode_response<T: DeserializeOwned>(message: CodexInbound) -> Result<T, String> {
    match message {
        CodexInbound::Response { result, .. } => serde_json::from_value(result)
            .map_err(|error| format!("decode Codex app-server response: {error}")),
        CodexInbound::Error { error, .. } => Err(format!(
            "Codex app-server error {}: {}",
            error.code, error.message
        )),
        CodexInbound::Notification { .. } | CodexInbound::ServerRequest { .. } => {
            Err("expected a Codex app-server response".into())
        }
    }
}
