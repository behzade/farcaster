use std::io::BufRead;

use serde_json::Value;

use super::{contract::OpenCodeEvent, transport::OpenCodeTcpTransport};

pub struct OpenCodeEventStream {
    reader: Box<dyn BufRead + Send>,
}

impl OpenCodeEventStream {
    pub fn connect(transport: &OpenCodeTcpTransport) -> Result<Self, String> {
        Ok(Self {
            reader: transport.open_event_stream()?,
        })
    }

    pub fn next(&mut self) -> Result<Option<OpenCodeEvent>, String> {
        read_event(&mut self.reader)
    }
}

pub fn read_event(reader: &mut impl BufRead) -> Result<Option<OpenCodeEvent>, String> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenCodeEventKind<'a> {
    CatalogUpdated,
    PermissionAsked,
    ExecutionStarted,
    ExecutionSucceeded,
    ExecutionInterrupted,
    ExecutionFailed,
    SessionTitleChanged,
    InboxDelivered,
    InboxCancelled,
    NextPrompted,
    TextStarted,
    TextDelta,
    TextEnded,
    ReasoningStarted,
    ReasoningDelta,
    ReasoningEnded,
    ToolInputStarted,
    ToolInputDelta,
    ToolInputEnded,
    ToolCalled,
    ToolProgress,
    ToolSucceeded,
    ToolFailed,
    StepStarted,
    StepEnded,
    UsageUpdated,
    RetryScheduled,
    StepFailed,
    CompactionStarted,
    CompactionEnded,
    CompactionFailed,
    FormCreated,
    KnownIgnored,
    Unknown(&'a str),
}

impl<'a> OpenCodeEventKind<'a> {
    pub(super) fn parse(event_type: &'a str) -> Self {
        let event_type = event_type
            .rsplit_once('.')
            .filter(|(_, suffix)| suffix.bytes().all(|byte| byte.is_ascii_digit()))
            .map_or(event_type, |(base, _)| base);
        match event_type {
            "catalog.updated" | "model.updated" => Self::CatalogUpdated,
            "permission.asked" => Self::PermissionAsked,
            "session.execution.started" => Self::ExecutionStarted,
            "session.execution.succeeded" => Self::ExecutionSucceeded,
            "session.execution.interrupted" => Self::ExecutionInterrupted,
            "session.execution.failed" => Self::ExecutionFailed,
            "session.updated" | "session.renamed" => Self::SessionTitleChanged,
            "session.inbox.delivered" => Self::InboxDelivered,
            "session.inbox.cancelled" => Self::InboxCancelled,
            "session.next.prompted" => Self::NextPrompted,
            "session.text.started" | "session.next.text.started" => Self::TextStarted,
            "session.text.delta" | "session.next.text.delta" => Self::TextDelta,
            "session.text.ended" | "session.next.text.ended" => Self::TextEnded,
            "session.reasoning.started" | "session.next.reasoning.started" => {
                Self::ReasoningStarted
            }
            "session.reasoning.delta" | "session.next.reasoning.delta" => Self::ReasoningDelta,
            "session.reasoning.ended" | "session.next.reasoning.ended" => Self::ReasoningEnded,
            "session.tool.input.started" | "session.next.tool.input.started" => {
                Self::ToolInputStarted
            }
            "session.tool.input.delta" | "session.next.tool.input.delta" => Self::ToolInputDelta,
            "session.tool.input.ended" | "session.next.tool.input.ended" => Self::ToolInputEnded,
            "session.tool.called" | "session.next.tool.called" => Self::ToolCalled,
            "session.tool.progress" | "session.next.tool.progress" => Self::ToolProgress,
            "session.tool.success" | "session.next.tool.success" => Self::ToolSucceeded,
            "session.tool.failed" | "session.next.tool.failed" => Self::ToolFailed,
            "session.step.started" | "session.next.step.started" => Self::StepStarted,
            "session.step.ended" | "session.next.step.ended" => Self::StepEnded,
            "session.usage.updated" | "session.usage.recorded" => Self::UsageUpdated,
            "session.retry.scheduled" => Self::RetryScheduled,
            "session.step.failed" => Self::StepFailed,
            "session.compaction.started" | "session.next.compaction.started" => {
                Self::CompactionStarted
            }
            "session.compaction.ended" | "session.next.compaction.ended" => Self::CompactionEnded,
            "session.compaction.failed" => Self::CompactionFailed,
            "form.created" => Self::FormCreated,
            "session.next.prompt.admitted"
            | "session.step.streamed"
            | "session.next.compaction.delta" => Self::KnownIgnored,
            _ => Self::Unknown(event_type),
        }
    }

    pub(super) fn is_execution(self) -> bool {
        matches!(
            self,
            Self::ExecutionStarted
                | Self::ExecutionSucceeded
                | Self::ExecutionInterrupted
                | Self::ExecutionFailed
                | Self::TextStarted
                | Self::TextDelta
                | Self::TextEnded
                | Self::ReasoningStarted
                | Self::ReasoningDelta
                | Self::ReasoningEnded
                | Self::ToolInputStarted
                | Self::ToolInputDelta
                | Self::ToolInputEnded
                | Self::ToolCalled
                | Self::ToolProgress
                | Self::ToolSucceeded
                | Self::ToolFailed
                | Self::StepStarted
                | Self::StepEnded
        ) || matches!(self, Self::Unknown(name) if name.starts_with("session.next.text.")
            || name.starts_with("session.next.reasoning.")
            || name.starts_with("session.next.tool.")
            || name.starts_with("session.next.step."))
    }

    pub(super) fn is_session_scoped(self) -> bool {
        match self {
            Self::CatalogUpdated => false,
            Self::Unknown(name) => name.starts_with("session.") || name == "form.created",
            _ => true,
        }
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
