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

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use serde_json::json;

    use super::*;

    #[test]
    fn parses_native_data_only_envelope() -> Result<(), String> {
        let input = b"data: {\"id\":\"event-1\",\"type\":\"session.text.delta\",\"data\":{\"sessionID\":\"session-1\",\"delta\":\"hello\"}}\n\n";
        let mut reader = BufReader::new(Cursor::new(input));

        let event = read_event(&mut reader)?.ok_or("expected event")?;
        assert_eq!(event.id.as_deref(), Some("event-1"));
        assert_eq!(event.event.as_deref(), Some("session.text.delta"));
        assert_eq!(
            event.data,
            json!({"sessionID": "session-1", "delta": "hello"})
        );
        assert!(read_event(&mut reader)?.is_none());
        Ok(())
    }

    #[test]
    fn preserves_unknown_events_and_multiline_data() -> Result<(), String> {
        let input = b": keepalive\nevent: future.event\ndata: first\ndata: second\n\n";
        let mut reader = BufReader::new(Cursor::new(input));

        let event = read_event(&mut reader)?.ok_or("expected event")?;
        assert_eq!(event.event.as_deref(), Some("future.event"));
        assert_eq!(event.data, Value::String("first\nsecond".into()));
        Ok(())
    }
}
