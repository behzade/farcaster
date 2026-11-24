use serde_json::{Value, json};
use std::path::Path;

// Prompt paths are untrusted. Use the submitted bytes.
pub(super) fn pasted_resources(message: &str) -> Option<Vec<Value>> {
    let (summary, _) = message.split_once("\n\n--- BEGIN PASTED FILE ")?;
    let (body, links) = summary
        .strip_prefix("Pasted text files:\n")
        .map(|links| ("", links))
        .or_else(|| summary.rsplit_once("\n\nPasted text files:\n"))?;
    let mut blocks = Vec::new();
    if !body.is_empty() {
        blocks.push(json!({"type":"text","text":body}));
    }
    let mut rest = &message[summary.len()..];
    for link in links.lines() {
        let (name, path) = link
            .strip_prefix("- [")?
            .strip_suffix(">)")?
            .split_once("](<")?;
        let uri = url::Url::from_file_path(Path::new(path)).ok()?;
        rest = rest.strip_prefix(&format!("\n\n--- BEGIN PASTED FILE {name} ---\n"))?;
        let (text, tail) = rest.split_once(&format!("\n--- END PASTED FILE {name} ---"))?;
        blocks.push(json!({"type":"resource","resource":{"uri":uri.as_str(),"mimeType":"text/plain","text":text}}));
        rest = tail;
    }
    rest.is_empty().then_some(blocks)
}
