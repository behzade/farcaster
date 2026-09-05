use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileAttachment {
    pub name: String,
    pub path: PathBuf,
}

/// Decode the legacy paste envelope only when the entire trailing list is valid.
/// Ordinary Markdown links in the user's prose are not attachments.
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
mod tests {
    use super::*;

    #[test]
    fn decodes_attachment_only_and_multiple_pastes() {
        let (text, files) = split_pasted_files(
            "Pasted text files:\n- [one.txt](</tmp/one.txt>)\n- [two.txt](</tmp/two.txt>)",
        );
        assert!(text.is_empty());
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, PathBuf::from("/tmp/two.txt"));
    }

    #[test]
    fn leaves_prose_and_malformed_lists_alone() {
        for text in [
            "See [one.txt](</tmp/one.txt>)",
            "Pasted text files:\n- ordinary prose",
            "Pasted text files:\n- [file](<https://example.com>)",
        ] {
            assert_eq!(split_pasted_files(text), (text, vec![]));
        }
    }
}
