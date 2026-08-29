//! File-backed text pastes keyed to each session composer.

use std::{
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::state::state_path;

const MAX_INLINE_PASTE_LINES: usize = 3;
static PASTE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerPaste {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) line_count: usize,
}

impl ComposerPaste {
    pub(crate) fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

impl FarcasterApp {
    pub(crate) fn has_composer_attachments(&self) -> bool {
        self.has_composer_images() || !self.current_composer_pastes().is_empty()
    }

    pub(crate) fn current_composer_pastes(&self) -> &[ComposerPaste] {
        self.composer_pastes
            .get(self.composer_sessions.current_target())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn paste_composer_text(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx
            .read_from_clipboard()
            .and_then(|clipboard| clipboard.text())
        else {
            return false;
        };
        let Some(paste) = store_long_paste(&text).ok().flatten() else {
            return false;
        };
        let target = self.composer_sessions.current_target().to_owned();
        self.composer_pastes.entry(target).or_default().push(paste);
        self.notify_composer(cx);
        true
    }

    pub(crate) fn remove_composer_paste(&mut self, index: usize, cx: &mut Context<Self>) {
        let target = self.composer_sessions.current_target().to_owned();
        if let Some(pastes) = self.composer_pastes.get_mut(&target)
            && index < pastes.len()
        {
            let paste = pastes.remove(index);
            let _ = std::fs::remove_file(paste.path);
            if pastes.is_empty() {
                self.composer_pastes.remove(&target);
            }
            self.notify_composer(cx);
        }
    }

    pub(crate) fn open_composer_paste(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .current_composer_pastes()
            .get(index)
            .map(|paste| paste.path.clone())
        else {
            return;
        };
        self.open_file_editor(path, window, cx);
    }

    pub(super) fn promote_composer_pastes(&mut self, from: &str, to: &str) {
        if let Some(pastes) = self.composer_pastes.remove(from) {
            self.composer_pastes
                .entry(to.to_owned())
                .or_default()
                .extend(pastes);
        }
    }
}

pub(super) fn append_pasted_files(message: &str, pastes: &[ComposerPaste]) -> String {
    if pastes.is_empty() {
        return message.to_owned();
    }
    let links = pastes
        .iter()
        .map(|paste| format!("- [{}](<{}>)", paste.file_name(), paste.path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let contents = pastes
        .iter()
        .map(|paste| {
            let name = paste.file_name();
            format!(
                "--- BEGIN PASTED FILE {name} ---\n{}\n--- END PASTED FILE {name} ---",
                paste.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let attachments = format!("Pasted text files:\n{links}\n\n{contents}");
    if message.is_empty() {
        attachments
    } else {
        format!("{message}\n\n{attachments}")
    }
}

fn store_long_paste(text: &str) -> Result<Option<ComposerPaste>, String> {
    let Some((normalized, line_count)) = long_paste(text) else {
        return Ok(None);
    };
    store_long_paste_in(&normalized, line_count, &paste_directory()?).map(Some)
}

fn long_paste(text: &str) -> Option<(String, usize)> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let line_count = normalized.lines().count().max(1);
    (line_count > MAX_INLINE_PASTE_LINES).then_some((normalized, line_count))
}

fn store_long_paste_in(
    normalized: &str,
    line_count: usize,
    directory: &Path,
) -> Result<ComposerPaste, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create paste directory {}: {error}", directory.display()))?;

    for _ in 0..100 {
        let path = directory.join(unique_paste_name());
        match open_private_file(&path) {
            Ok(mut file) => {
                file.write_all(normalized.as_bytes())
                    .map_err(|error| format!("write pasted text {}: {error}", path.display()))?;
                return Ok(ComposerPaste {
                    path,
                    content: normalized.to_owned(),
                    line_count,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "create pasted text file {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err("could not allocate a unique pasted text file".to_owned())
}

fn open_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn paste_directory() -> Result<PathBuf, String> {
    state_path()?
        .parent()
        .map(|parent| parent.join("pastes"))
        .ok_or_else(|| "GUI state database has no parent directory".to_owned())
}

fn unique_paste_name() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = PASTE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("pasted-text-{timestamp}-{sequence}.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pastes_longer_than_three_lines_become_files() -> Result<(), String> {
        assert!(long_paste("one\ntwo\nthree").is_none());
        assert!(long_paste("one\ntwo\nthree\n").is_none());
        let (normalized, line_count) = long_paste("one\r\ntwo\rthree\nfour")
            .ok_or_else(|| "expected a long paste".to_owned())?;
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paste = store_long_paste_in(&normalized, line_count, directory.path())?;
        assert_eq!(paste.line_count, 4);
        assert_eq!(
            std::fs::read_to_string(&paste.path).map_err(|error| error.to_string())?,
            "one\ntwo\nthree\nfour"
        );
        Ok(())
    }
}
