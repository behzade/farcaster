use super::super::{child_stderr, farcaster_mcp};
use crate::agents::{AgentLaunchConfig, HarnessAccessMode};
use claude_sdk_types::{
    SDKControlRequest, SDKControlRequestInner, SDKControlResponse, SDKUserMessage, StdoutMessage,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

pub(super) fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Claude protocol: {error}"))
}

fn decode_frame(line: &str) -> Result<Option<StdoutMessage>, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("read Claude JSON: {error}"))?;
    // SDKControlInterruptRequest documents these native queue-status frames,
    // but the SDK's StdoutMessage union omits them. Its runtime forwards them
    // without validation. Our local queue does not consume native queue status.
    if value["type"] == "command_lifecycle" {
        return Ok(None);
    }
    let kind = value["type"].as_str().unwrap_or("unknown").to_owned();
    let subtype = value["subtype"].as_str().unwrap_or("-").to_owned();
    serde_json::from_value(value).map(Some).map_err(|error| {
        // Report version drift without putting prompts, tool data or secrets in logs.
        format!("Claude CLI frame {kind}/{subtype} does not match SDK 0.3.257: {error}. Native session files remain intact.")
    })
}

pub(super) struct Process {
    child: Child,
    input: Option<ChildStdin>,
    incoming: Receiver<Result<StdoutMessage, String>>,
    pending: VecDeque<StdoutMessage>,
    next_id: u64,
}

pub(super) fn configure(
    command: &mut std::process::Command,
    access: HarnessAccessMode,
    session_id: &str,
    resume: bool,
    caller: Option<&str>,
    persist: bool,
) {
    command.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--replay-user-messages",
        "--permission-prompt-tool",
        "stdio",
        "--setting-sources=user,project,local",
    ]);
    command.arg(format!(
        "--{}={session_id}",
        if resume { "resume" } else { "session-id" }
    ));
    command.arg(format!("--permission-mode={}", permission_mode(access)));
    if access == HarnessAccessMode::Full {
        command.arg("--allow-dangerously-skip-permissions");
    }
    if !persist {
        command.arg("--no-session-persistence");
    }
    if let Some(token) = caller.filter(|_| farcaster_mcp::enabled()) {
        command.arg("--mcp-config").arg(
            json!({"mcpServers": {"farcaster": {
                "type": "http", "url": farcaster_mcp::URL,
                "headers": {farcaster_mcp::CALLER_HEADER: token}
            }}})
            .to_string(),
        );
    }
}

pub(super) fn permission_mode(access: HarnessAccessMode) -> &'static str {
    if access == HarnessAccessMode::Full {
        "bypassPermissions"
    } else {
        "default"
    }
}

impl Process {
    pub(super) fn spawn(
        config: &AgentLaunchConfig,
        project: &Path,
        id: &str,
        resume: bool,
        caller: Option<&str>,
        wake: Option<thread::Thread>,
        persist: bool,
    ) -> Result<Self, String> {
        let mut command = config.command(project)?;
        configure(
            &mut command,
            config.access_mode,
            id,
            resume,
            caller,
            persist,
        );
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start Claude CLI: {error}"))?;
        let (tx, incoming) = mpsc::sync_channel(256);
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or("Claude stdout missing")?;
        let mut process = Self {
            child,
            input,
            incoming,
            pending: VecDeque::new(),
            next_id: 0,
        };
        child_stderr::capture(&mut process.child, "claude")?;
        thread::Builder::new()
            .name("claude-stdout".into())
            .spawn(move || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut line = String::new();
                    let result = match reader.by_ref().take(16 * 1024 * 1024).read_line(&mut line) {
                        Ok(0) => Err("Claude CLI closed stdout".into()),
                        Ok(_) if !line.ends_with('\n') => {
                            Err("Claude CLI frame is incomplete or exceeds 16 MiB".into())
                        }
                        Ok(_) => decode_frame(&line),
                        Err(error) => Err(format!("read Claude CLI: {error}")),
                    };
                    let result = match result {
                        Ok(Some(frame)) => Ok(frame),
                        Ok(None) => continue,
                        Err(error) => Err(error),
                    };
                    let failed = result.is_err();
                    if tx.send(result).is_err() {
                        break;
                    }
                    if let Some(wake) = &wake {
                        wake.unpark();
                    }
                    if failed {
                        break;
                    }
                }
            })
            .map_err(|error| format!("start Claude reader: {error}"))?;
        Ok(process)
    }
    fn write(&mut self, message: &impl Serialize) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("Claude CLI is closed")?;
        serde_json::to_writer(&mut *input, message).map_err(|error| error.to_string())?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|error| format!("write Claude CLI: {error}"))
    }
    pub(super) fn prompt(&mut self, message: SDKUserMessage) -> Result<(), String> {
        self.write(&message)
    }
    pub(super) fn reply(&mut self, message: SDKControlResponse) -> Result<(), String> {
        self.write(&message)
    }
    pub(super) fn request(&mut self, request: Value) -> Result<String, String> {
        let request: SDKControlRequestInner = decode(request)?;
        self.next_id += 1;
        let id = format!("farcaster-{}", self.next_id);
        let envelope: SDKControlRequest =
            decode(json!({"type":"control_request", "request_id":id, "request":request}))?;
        self.write(&envelope)?;
        Ok(id)
    }
    pub(super) fn wait(&mut self, request: Value) -> Result<Value, String> {
        let id = self.request(request)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let frame = self
                .incoming
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| format!("Claude control request {id}: {error}"))??;
            if let StdoutMessage::SDKControlResponse(response) = &frame {
                let body =
                    serde_json::to_value(&response.response).map_err(|error| error.to_string())?;
                if body["request_id"].as_str() == Some(&id) {
                    if body["subtype"] == "error" {
                        return Err(body["error"]
                            .as_str()
                            .unwrap_or("Claude rejected request")
                            .into());
                    }
                    return Ok(body.get("response").cloned().unwrap_or_else(|| json!({})));
                }
            }
            self.pending.push_back(frame);
        }
    }
    pub(super) fn poll(&mut self) -> Option<Result<StdoutMessage, String>> {
        self.pending
            .pop_front()
            .map(Ok)
            .or_else(|| match self.incoming.try_recv() {
                Ok(frame) => Some(frame),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("Claude stdout reader stopped".into())),
            })
    }
    pub(super) fn close(&mut self) -> Result<(), String> {
        self.input.take();
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            self.child.kill().map_err(|error| error.to_string())?;
        }
        self.child
            .wait()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
