use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::repository::git_head_contents;
use gpui::{Context, Window};

use super::FarcasterApp;

pub(super) fn prepare_diff(
    project: PathBuf,
    target: String,
    path: PathBuf,
    editor: &'static str,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
    open: impl FnOnce(
        &mut FarcasterApp,
        tempfile::NamedTempFile,
        &mut Window,
        &mut Context<FarcasterApp>,
    ) -> Result<(), String>
    + 'static,
) {
    let prepared = cx
        .background_executor()
        .spawn(async move { head_tempfile(&path, editor) });
    cx.spawn_in(window, async move |weak, cx| {
        let prepared = prepared.await;
        let _ = weak.update_in(cx, |app, window, cx| {
            if app.workspace_project() != project
                || app.composer.sessions.current_target() != target
            {
                return;
            }
            let result = if app.project.repository.execution_allowed {
                prepared.and_then(|base| open(app, base, window, cx))
            } else {
                Err("Project trust changed while preparing the diff.".into())
            };
            if let Err(error) = result {
                app.notify_workspace_error(editor, error, cx);
            }
        });
    })
    .detach();
}

pub(super) fn location(path: &Path, line: Option<u64>) -> String {
    match line {
        Some(line) => format!("{}:{}", path.display(), line.max(1)),
        None => path.to_string_lossy().into_owned(),
    }
}

fn head_tempfile(path: &Path, editor: &str) -> Result<tempfile::NamedTempFile, String> {
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

#[cfg(test)]
#[path = "external_editor_tests.rs"]
mod tests;
