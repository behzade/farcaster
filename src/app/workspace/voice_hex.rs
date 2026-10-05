use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};

pub(super) enum Event {
    Recording,
    Transcript(String),
    Failed(String),
}

pub(super) fn available() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    Endpoint::discover()
        .and_then(|endpoint| endpoint.request("GET", "/capabilities", None, Value::Null))
        .is_ok_and(|capabilities| capabilities["serviceCapture"] == true)
}

enum Control {
    Finish,
    Cancel,
}

pub(super) struct Dictation(mpsc::Sender<Control>);

impl Dictation {
    #[cfg(test)]
    pub(super) fn stub() -> Self {
        Self(mpsc::channel().0)
    }

    #[cfg(test)]
    pub(super) fn stub_with_finish() -> (Self, impl Fn() -> bool) {
        let (send, receive) = mpsc::channel();
        (Self(send), move || {
            matches!(receive.try_recv(), Ok(Control::Finish))
        })
    }

    pub(super) fn start() -> Result<(Self, async_channel::Receiver<Event>), String> {
        let (control, commands) = mpsc::channel();
        let (updates, receiver) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("hex-dictation".into())
            .spawn(move || {
                let result = record(commands, &updates);
                if let Err(error) = result {
                    let _ = updates.send_blocking(Event::Failed(error));
                }
            })
            .map_err(|error| format!("Start dictation: {error}"))?;
        Ok((Self(control), receiver))
    }

    pub(super) fn finish(&self) {
        let _ = self.0.send(Control::Finish);
    }
}

impl Drop for Dictation {
    fn drop(&mut self) {
        let _ = self.0.send(Control::Cancel);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Endpoint {
    port: u16,
    token: String,
    api_version: String,
}

impl Endpoint {
    fn discover() -> Result<Self, String> {
        let directory = match std::env::var_os("HEX_APPLICATION_SUPPORT_DIR") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?)
                .join("Library/Application Support/voice-control"),
        };
        let bytes = std::fs::read(directory.join("local-api.json")).map_err(|_| {
            "Open the Rust version of Hex to use voice. The older Swift Hex app does not support dictation requests.".to_owned()
        })?;
        let endpoint: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Read Hex connection: {error}"))?;
        if endpoint.api_version != "2" {
            return Err("This Hex API version is not supported. Update Hex.".into());
        }
        Ok(endpoint)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        owner: Option<&str>,
        body: Value,
    ) -> Result<Value, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|error| format!("Connect to Hex: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(if path.ends_with("/finish") {
                125
            } else {
                5
            })))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        // Tokens come from Hex's discovery/start responses, never from a shell command.
        if [self.token.as_str(), owner.unwrap_or("")]
            .iter()
            .any(|token| token.contains(['\r', '\n']))
        {
            return Err("Invalid Hex connection token".into());
        }
        let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", self.port, self.token, body.len()).map_err(|e| e.to_string())?;
        if let Some(owner) = owner {
            write!(stream, "X-Hex-Dictation-Token: {owner}\r\n").map_err(|e| e.to_string())?;
        }
        stream
            .write_all(b"\r\n")
            .and_then(|_| stream.write_all(&body))
            .map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        stream
            .take(1024 * 1024 + 1)
            .read_to_end(&mut response)
            .map_err(|e| format!("Read Hex response: {e}"))?;
        parse_response(&response)
    }
}

fn parse_response(response: &[u8]) -> Result<Value, String> {
    if response.len() > 1024 * 1024 {
        return Err("Hex response is too large".into());
    }
    let boundary = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .ok_or("Invalid Hex response")?;
    let headers = std::str::from_utf8(&response[..boundary]).map_err(|e| e.to_string())?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("Missing Hex response status")?;
    let bytes = &response[boundary + 4..];
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes).map_err(|e| format!("Read Hex JSON: {e}"))?
    };
    if !(200..300).contains(&status) {
        return Err(format!(
            "Hex: {} (HTTP {status})",
            body["code"].as_str().unwrap_or("request failed")
        ));
    }
    Ok(body)
}

fn record(
    commands: mpsc::Receiver<Control>,
    updates: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let endpoint = Endpoint::discover()?;
    record_with_endpoint(&endpoint, commands, updates)
}

fn record_with_endpoint(
    endpoint: &Endpoint,
    commands: mpsc::Receiver<Control>,
    updates: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let capabilities = endpoint.request("GET", "/capabilities", None, Value::Null)?;
    if capabilities["serviceCapture"] != true {
        return Err(
            "This Hex instance does not support microphone capture. Open Hex desktop.".into(),
        );
    }
    let started = endpoint.request("POST", "/dictations", None, json!({"source": "Farcaster"}))?;
    let id = started["id"]
        .as_u64()
        .ok_or("Hex did not return a recording ID")?;
    let owner = started["ownerToken"]
        .as_str()
        .ok_or("Hex did not return a recording token")?;
    let path = format!("/dictations/{id}");
    let _ = updates.send_blocking(Event::Recording);
    let result = loop {
        match commands.recv_timeout(Duration::from_secs(3)) {
            Ok(Control::Finish) => {
                let result = endpoint
                    .request("POST", &format!("{path}/finish"), Some(owner), Value::Null)
                    .and_then(|reply| {
                        reply["transcript"]
                            .as_str()
                            .map(str::to_owned)
                            .ok_or("Hex did not return a transcript".into())
                    });
                break result.map(|text| {
                    let _ = updates.send_blocking(Event::Transcript(text));
                });
            }
            Ok(Control::Cancel) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = endpoint.request(
                    "POST",
                    &format!("{path}/heartbeat"),
                    Some(owner),
                    Value::Null,
                ) {
                    break Err(error);
                }
            }
        }
    };
    // Also release recording after a failed finish or heartbeat.
    let _ = endpoint.request("POST", &format!("{path}/cancel"), Some(owner), Value::Null);
    result
}

#[cfg(test)]
#[path = "voice_hex_tests.rs"]
mod tests;
