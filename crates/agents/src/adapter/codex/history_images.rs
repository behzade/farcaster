use std::{fs::File, io::BufRead, path::Path};

use base64::Engine as _;
use serde_json::Value;

struct Shortened {
    prefix: String,
    suffix: String,
    length: usize,
    image: Option<(String, String)>,
    ambiguous: bool,
}

impl Shortened {
    fn parse(data: &str) -> Option<Self> {
        let (prefix, rest) = data.split_once('…')?;
        let (count, suffix) = rest.split_once(" chars truncated…")?;
        let omitted = count.parse::<usize>().ok()?;
        if prefix.is_empty() || suffix.is_empty() || !prefix.is_ascii() || !suffix.is_ascii() {
            return None;
        }
        Some(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
            length: prefix
                .len()
                .checked_add(omitted)?
                .checked_add(suffix.len())?,
            image: None,
            ambiguous: false,
        })
    }

    fn consider(&mut self, data: &str, mime: &str) {
        if self.ambiguous
            || data.len() != self.length
            || !data.starts_with(&self.prefix)
            || !data.ends_with(&self.suffix)
            || self
                .image
                .as_ref()
                .is_some_and(|(previous, _)| previous == data)
            || base64::engine::general_purpose::STANDARD
                .decode(data)
                .is_err()
        {
            return;
        }
        if self.image.is_some() {
            self.ambiguous = true;
        } else {
            self.image = Some((data.into(), mime.into()));
        }
    }
}

fn shortened_images(messages: &[Value]) -> Vec<(usize, usize, Shortened)> {
    let mut missing = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        if message["role"] != "toolResult" {
            continue;
        }
        for (block_index, block) in message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if block["type"] == "image"
                && let Some(shortened) = block["data"].as_str().and_then(Shortened::parse)
            {
                missing.push((message_index, block_index, shortened));
            }
        }
    }
    missing
}

pub(super) fn needs_restore(messages: &[Value]) -> bool {
    !shortened_images(messages).is_empty()
}

// Codex can shorten image bytes inside an event's serialized MCP result while
// retaining the full image in the model-facing output. Read only that thread's
// rollout, and restore only a unique match for the preserved bytes and length.
pub(super) fn restore(messages: &mut [Value], thread: &Value, home: &Path, locator: &str) {
    let mut missing = shortened_images(messages);
    let Some(shortest) = missing.iter().map(|(_, _, image)| image.length).min() else {
        return;
    };
    let mut read = || -> Option<()> {
        let home = home.canonicalize().ok()?;
        let path = Path::new(thread["path"].as_str()?).canonicalize().ok()?;
        if !path.starts_with(&home) {
            return None;
        }
        let mut reader = std::io::BufReader::new(File::open(path).ok()?);
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let header: Value = serde_json::from_str(&line).ok()?;
        if header["type"] != "session_meta" || header["payload"]["id"] != locator {
            return None;
        }
        loop {
            line.clear();
            if reader.read_line(&mut line).ok()? == 0 {
                break;
            }
            // JSON escaping can only make an ASCII base64 string longer on disk.
            if line.len() < shortest {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if record["type"] != "response_item"
                || !matches!(
                    record["payload"]["type"].as_str(),
                    Some("custom_tool_call_output" | "function_call_output")
                )
            {
                continue;
            }
            for block in record["payload"]["output"].as_array().into_iter().flatten() {
                if block["type"] != "input_image" {
                    continue;
                }
                let Some((mime, data)) = block["image_url"]
                    .as_str()
                    .and_then(|url| url.strip_prefix("data:"))
                    .and_then(|url| url.split_once(";base64,"))
                else {
                    continue;
                };
                if !mime.starts_with("image/") {
                    continue;
                }
                for (_, _, shortened) in &mut missing {
                    shortened.consider(data, mime);
                }
            }
        }
        Some(())
    };
    if read().is_none() {
        return;
    }
    for (message, block, shortened) in missing {
        if !shortened.ambiguous
            && let Some((data, mime)) = shortened.image
        {
            messages[message]["content"][block]["data"] = data.into();
            messages[message]["content"][block]["mimeType"] = mime.into();
        }
    }
}

#[cfg(test)]
#[path = "history_images_tests.rs"]
mod tests;
