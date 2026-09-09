use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use agent_client_protocol::{Agent, Client, ConnectionTo, Lines, Responder, UntypedMessage};
use futures::{
    FutureExt as _,
    channel::oneshot,
    io::{
        AsyncBufRead, AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader,
    },
};
use serde_json::{Value, json};

use super::{
    AcpProfile,
    events::{AcpInbound, AcpRequestId},
};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Bridges SDK callbacks to Farcaster's worker polling API. The SDK owns
/// encoding, request IDs, correlation, and dispatch. Raw payloads preserve
/// Cursor extensions and provider metadata for our translators.
pub(super) struct AcpConnection {
    connection: ConnectionTo<Agent>,
    incoming: mpsc::Receiver<Result<AcpInbound, String>>,
    queued: VecDeque<AcpInbound>,
    sender: EventSender,
    responders: Arc<Mutex<HashMap<AcpRequestId, Responder>>>,
    shutdown: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
struct EventSender {
    sender: mpsc::Sender<Result<AcpInbound, String>>,
    wake: Option<thread::Thread>,
}

impl EventSender {
    fn send(&self, event: Result<AcpInbound, String>) -> agent_client_protocol::Result<()> {
        self.sender
            .send(event)
            .map_err(agent_client_protocol::Error::into_internal_error)?;
        if let Some(wake) = &self.wake {
            wake.unpark();
        }
        Ok(())
    }
}

impl AcpConnection {
    pub(super) fn new(
        reader: impl AsyncRead + Unpin + Send + 'static,
        writer: impl AsyncWrite + Unpin + Send + 'static,
        wake: Option<thread::Thread>,
    ) -> Result<Self, String> {
        let incoming_lines =
            futures::stream::try_unfold(BufReader::new(reader), |mut reader| async {
                Ok(read_frame(&mut reader, MAX_FRAME_BYTES)
                    .await?
                    .map(|line| (line, reader)))
            });
        let outgoing_lines = futures::sink::unfold(writer, |mut writer, line: String| async move {
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            Ok::<_, io::Error>(writer)
        });
        let (sender, incoming) = mpsc::channel();
        let sender = EventSender { sender, wake };
        let responders = Arc::new(Mutex::new(HashMap::new()));
        let request_responders = Arc::clone(&responders);
        let request_events = sender.clone();
        let notification_events = sender.clone();
        let terminal_events = sender.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (shutdown, stopped) = oneshot::channel();
        thread::Builder::new()
            .name("acp-sdk".into())
            .spawn(move || {
                let client = Client.builder()
                    .name("farcaster")
                    .on_receive_request(
                        async move |request: UntypedMessage, responder, _cx| {
                            let id = responder.id().clone();
                            request_responders
                                .lock()
                                .map_err(agent_client_protocol::Error::into_internal_error)?
                                .insert(id.clone(), responder);
                            let (method, params) = request.into_parts();
                            request_events.send(Ok(AcpInbound::AgentRequest { id, method, params }))
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_notification(
                        async move |notification: UntypedMessage, _cx| {
                            let (method, params) = notification.into_parts();
                            notification_events.send(Ok(AcpInbound::Notification { method, params }))
                        },
                        agent_client_protocol::on_receive_notification!(),
                    );
                let connection = client.connect_with(
                    Lines::new(outgoing_lines, incoming_lines),
                    async move |cx| {
                        ready_tx
                            .send(cx.clone())
                            .map_err(agent_client_protocol::Error::into_internal_error)?;
                        futures::select! {
                            _ = stopped.fuse() => Ok(()),
                            _ = cx.incoming_closed().fuse() => Err(
                                agent_client_protocol::util::internal_error("ACP agent closed its output")
                            ),
                        }
                    },
                );
                let result = futures::executor::block_on(connection);
                if let Err(error) = result {
                    let _ = terminal_events.send(Err(error.to_string()));
                }
            })
            .map_err(|error| format!("start ACP connection: {error}"))?;
        let connection = ready_rx
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|error| format!("start ACP connection: {error}"))?;
        Ok(Self {
            connection,
            incoming,
            queued: VecDeque::new(),
            sender,
            responders,
            shutdown: Some(shutdown),
        })
    }

    pub(super) fn initialize(&self, profile: &AcpProfile) -> Result<Value, String> {
        let initialized = self.request_blocking("initialize", json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "_meta": {"parameterizedModelPicker": profile.backend == "cursor-cli"},
                "fs": {"readTextFile": false, "writeTextFile": false},
                "terminal": false,
            },
            "clientInfo": {"name": "farcaster", "title": "Farcaster", "version": env!("CARGO_PKG_VERSION")},
        }))?;
        if initialized.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
            return Err(format!(
                "{} ACP agent did not negotiate protocol version 1",
                profile.name
            ));
        }
        if let Some(method_id) = profile.auth_method {
            if !initialized
                .get("authMethods")
                .and_then(Value::as_array)
                .is_some_and(|methods| {
                    methods
                        .iter()
                        .any(|method| method.get("id").and_then(Value::as_str) == Some(method_id))
                })
            {
                return Err(format!(
                    "{} ACP agent did not advertise authentication method {method_id}",
                    profile.name
                ));
            }
            self.request_blocking("authenticate", json!({"methodId": method_id}))?;
        }
        Ok(initialized)
    }

    pub(super) fn request_blocking(&self, method: &str, params: Value) -> Result<Value, String> {
        let request = UntypedMessage::new(method, params).map_err(|error| error.to_string())?;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.connection
            .send_request(request)
            .on_receiving_result(move |result| async move {
                let _ = sender.send(result);
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|error| format!("wait for ACP {method}: {error}"))?
            .map_err(|error| error.to_string())
    }

    pub(super) fn model_catalog(&self, profile: &AcpProfile) -> Result<Vec<Value>, String> {
        if profile.backend != "cursor-cli" {
            return Ok(Vec::new());
        }
        let result = self.request_blocking("cursor/list_available_models", json!({}))?;
        result
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "Cursor model catalog omitted models".into())
    }

    pub(super) fn send_request(&self, method: &str, params: Value) -> Result<AcpRequestId, String> {
        let request = self
            .connection
            .send_request(UntypedMessage::new(method, params).map_err(|error| error.to_string())?);
        let id = request.id().clone();
        let response_id = id.clone();
        let sender = self.sender.clone();
        request
            .on_receiving_result(move |result| async move {
                sender.send(Ok(match result {
                    Ok(result) => AcpInbound::Response {
                        id: response_id,
                        result,
                    },
                    Err(error) => AcpInbound::Error {
                        id: response_id,
                        message: error.to_string(),
                    },
                }))
            })
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    pub(super) fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.connection
            .send_notification(
                UntypedMessage::new(method, params).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
    }

    pub(super) fn respond(&self, id: &AcpRequestId, result: Value) -> Result<(), String> {
        self.responders
            .lock()
            .map_err(|error| error.to_string())?
            .remove(id)
            .ok_or_else(|| format!("unknown ACP request: {id}"))?
            .respond(result)
            .map_err(|error| error.to_string())
    }

    pub(super) fn poll(&mut self) -> Option<Result<AcpInbound, String>> {
        self.queued
            .pop_front()
            .map(Ok)
            .or_else(|| self.incoming.try_recv().ok())
    }

    pub(super) fn drain_queued(&mut self) -> Result<VecDeque<AcpInbound>, String> {
        self.queued
            .drain(..)
            .map(Ok)
            .chain(self.incoming.try_iter())
            .collect()
    }

    pub(super) fn restore_queued(&mut self, queued: VecDeque<AcpInbound>) {
        self.queued.extend(queued);
    }
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

/// Bound each frame before allocating it or handing it to the SDK parser.
async fn read_frame(
    reader: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> io::Result<Option<String>> {
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            break;
        }
        let length = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if length > limit.saturating_sub(frame.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ACP frame exceeds size limit",
            ));
        }
        frame.extend_from_slice(&available[..length]);
        reader.consume_unpin(length);
        if frame.last() == Some(&b'\n') {
            break;
        }
    }
    if frame.is_empty() {
        return Ok(None);
    }
    while matches!(frame.last(), Some(b'\n' | b'\r')) {
        frame.pop();
    }
    String::from_utf8(frame)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
