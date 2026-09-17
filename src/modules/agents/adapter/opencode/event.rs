use std::io::BufRead;

use serde_json::Value;

use super::{contract::OpenCodeEvent, transport::OpenCodeTcpTransport};

pub(crate) struct OpenCodeEventStream {
    reader: Box<dyn BufRead + Send>,
}

impl OpenCodeEventStream {
    pub(crate) fn connect(transport: &OpenCodeTcpTransport) -> Result<Self, String> {
        Ok(Self {
            reader: transport.open_event_stream()?,
        })
    }

    pub(crate) fn next(&mut self) -> Result<Option<OpenCodeEvent>, String> {
        read_event(&mut self.reader)
    }
}

pub(crate) fn read_event(reader: &mut impl BufRead) -> Result<Option<OpenCodeEvent>, String> {
    let mut id = None;
    let mut event = None;
    let mut data = Vec::new();
    let mut saw_field = false;

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("read OpenCode event stream: {error}"))?;
        if read == 0 {
            if !saw_field {
                return Ok(None);
            }
            break;
        }

        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if saw_field {
                break;
            }
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        saw_field = true;

        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "id" => id = Some(value.to_owned()),
            "event" => event = Some(value.to_owned()),
            "data" => data.push(value.to_owned()),
            _ => {}
        }
    }

    let data = data.join("\n");
    let mut data = serde_json::from_str(&data).unwrap_or(Value::String(data));
    if let Value::Object(envelope) = &data {
        let native_type = envelope.get("type").and_then(Value::as_str);
        let native_data = envelope.get("data");
        if let (Some(native_type), Some(native_data)) = (native_type, native_data) {
            if id.is_none() {
                id = envelope
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if event.is_none() {
                event = Some(native_type.to_owned());
            }
            data = native_data.clone();
        }
    }
    Ok(Some(OpenCodeEvent { id, event, data }))
}

/// Maps native event names onto the stable vocabulary consumed by the adapter.
/// OpenCode may version events with a numeric suffix or rename them between releases.
pub(super) fn normalized_event_type(event_type: &str) -> &str {
    let event_type = event_type
        .rsplit_once('.')
        .filter(|(_, suffix)| suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(event_type, |(base, _)| base);
    // Alias only events with equivalent payloads and semantics. Unique
    // session.next lifecycle events remain distinct in the worker dispatch.
    match event_type {
        "model.updated" => "catalog.updated",
        "session.next.text.started" => "session.text.started",
        "session.next.text.delta" => "session.text.delta",
        "session.next.text.ended" => "session.text.ended",
        "session.next.reasoning.started" => "session.reasoning.started",
        "session.next.reasoning.delta" => "session.reasoning.delta",
        "session.next.reasoning.ended" => "session.reasoning.ended",
        "session.next.tool.input.started" => "session.tool.input.started",
        "session.next.tool.input.delta" => "session.tool.input.delta",
        "session.next.tool.input.ended" => "session.tool.input.ended",
        "session.next.tool.called" => "session.tool.called",
        "session.next.tool.progress" => "session.tool.progress",
        "session.next.tool.success" => "session.tool.success",
        "session.next.tool.failed" => "session.tool.failed",
        "session.next.step.started" => "session.step.started",
        "session.next.step.ended" => "session.step.ended",
        "session.next.compaction.started" => "session.compaction.started",
        "session.next.compaction.ended" => "session.compaction.ended",
        _ => event_type,
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
