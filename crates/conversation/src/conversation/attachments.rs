use std::path::PathBuf;

const PASTED_FILE_START: &str = "\n\n--- BEGIN PASTED FILE ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAttachment {
    pub name: String,
    pub path: PathBuf,
}

pub(super) fn split_pasted_files(message: &str) -> (&str, Vec<FileAttachment>) {
    let summary = pasted_file_summary(message);
    let (body, links) = if let Some(links) = summary.strip_prefix("Pasted text files:\n") {
        ("", links)
    } else if let Some(parts) = summary.rsplit_once("\n\nPasted text files:\n") {
        parts
    } else {
        return (summary, Vec::new());
    };
    let files: Option<Vec<_>> = links
        .lines()
        .map(|line| {
            let (name, path) = line
                .strip_prefix("- [")?
                .strip_suffix(">)")?
                .split_once("](<")?;
            if name.is_empty() || !std::path::Path::new(path).is_absolute() {
                return None;
            }
            Some(FileAttachment {
                name: "Pasted text".into(),
                path: path.into(),
            })
        })
        .collect();
    match files {
        Some(files) if !files.is_empty() => (body, files),
        _ => (summary, Vec::new()),
    }
}

pub(super) fn pasted_file_summary(message: &str) -> &str {
    message
        .split_once(PASTED_FILE_START)
        .map_or(message, |(summary, _)| summary)
}

pub(super) fn pasted_file_summary_length<'a>(
    parts: impl Iterator<Item = &'a str>,
) -> Option<usize> {
    let marker = PASTED_FILE_START.as_bytes();
    let mut tail = [0; PASTED_FILE_START.len() - 1];
    let mut tail_len = 0;
    let mut offset = 0;
    for part in parts {
        let bytes = part.as_bytes();
        let head_len = bytes.len().min(tail.len());
        let mut boundary = [0; 2 * (PASTED_FILE_START.len() - 1)];
        boundary[..tail_len].copy_from_slice(&tail[..tail_len]);
        boundary[tail_len..tail_len + head_len].copy_from_slice(&bytes[..head_len]);
        let boundary = &boundary[..tail_len + head_len];
        if let Some(index) = boundary
            .windows(marker.len())
            .position(|bytes| bytes == marker)
        {
            return Some(offset - tail_len + index);
        }
        if let Some(index) = part.find(PASTED_FILE_START) {
            return Some(offset + index);
        }
        if bytes.len() >= tail.len() {
            tail_len = tail.len();
            tail.copy_from_slice(&bytes[bytes.len() - tail_len..]);
        } else {
            tail_len = boundary.len().min(tail.len());
            tail[..tail_len].copy_from_slice(&boundary[boundary.len() - tail_len..]);
        }
        offset += bytes.len();
    }
    None
}

// Adapters expose output files as resource links; native IDs stay behind the adapter.
pub(super) fn result_files(content: &serde_json::Value) -> Vec<FileAttachment> {
    content_blocks(content)
        .into_iter()
        .flatten()
        .filter_map(|block| {
            if block.get("type")?.as_str()? != "resource_link" {
                return None;
            }
            let uri = block.get("uri")?.as_str()?;
            let path = match url::Url::parse(uri) {
                Ok(url) if url.scheme() == "file" => url.to_file_path().ok()?,
                Ok(url) if matches!(url.scheme(), "http" | "https") => {
                    std::path::PathBuf::from(uri)
                }
                Ok(_) => return None,
                Err(_) => {
                    let path = std::path::PathBuf::from(uri);
                    if !path.is_absolute() {
                        return None;
                    }
                    path
                }
            };
            Some(FileAttachment {
                name: block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }),
                path,
            })
        })
        .collect()
}

pub(super) fn content_blocks(value: &serde_json::Value) -> Option<&[serde_json::Value]> {
    value
        .as_array()
        .or_else(|| value.get("content")?.as_array())
        .map(Vec::as_slice)
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
