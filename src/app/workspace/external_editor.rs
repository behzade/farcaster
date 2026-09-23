use std::{
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};

use crate::repository::git_head_contents;

pub(super) fn head_tempfile(path: &Path, editor: &str) -> Result<tempfile::NamedTempFile, String> {
    let suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let mut base = tempfile::Builder::new()
        .prefix("farcaster-head-")
        .suffix(&suffix)
        .tempfile()
        .map_err(|error| format!("prepare {editor} diff: {error}"))?;
    base.write_all(&git_head_contents(path)?)
        .map_err(|error| format!("prepare {editor} diff: {error}"))?;
    Ok(base)
}

pub(super) fn launch(
    program: &Path,
    editor: &'static str,
    project: &Path,
    arguments: &[String],
    temporary: Option<tempfile::NamedTempFile>,
) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(arguments)
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start {editor} ({}): {error}", program.display()))?;
    std::thread::spawn(move || {
        match child.wait() {
            Ok(status) if !status.success() => {
                zlog::warn!("{editor} command exited with {status}");
            }
            Err(error) => {
                zlog::warn!("{editor} command failed: {error}");
            }
            _ => {}
        }
        drop(temporary);
    });
    Ok(())
}
