use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileAttachment {
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
        .split_once("\n\n--- BEGIN PASTED FILE ")
        .map_or(message, |(summary, _)| summary)
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
