use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::repository::git_head_contents;
use gpui::{Context, Window};

use super::{
    FarcasterApp,
    editor::{EditorBackend, EditorRequest},
};

pub(super) struct VsCodeBackend;

impl EditorBackend for VsCodeBackend {
    fn name(&self) -> &'static str {
        "VS Code"
    }

    fn open(
        &self,
        _app: &mut FarcasterApp,
        request: EditorRequest,
        _window: &mut Window,
        _cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String> {
        match request {
            EditorRequest::Project(project) => open_project(&project),
            EditorRequest::File {
                project,
                path,
                line,
                diff,
            } => {
                if diff {
                    open_diff(&project, &path)
                } else {
                    open_locations(&project, &[(path, line)])
                }
            }
            EditorRequest::Review {
                project, locations, ..
            } => open_locations(&project, &locations),
        }
    }
}

fn open_project(project: &Path) -> Result<(), String> {
    launch(project, &[project.to_string_lossy().into_owned()], None)
}

fn open_locations(project: &Path, locations: &[(PathBuf, Option<u64>)]) -> Result<(), String> {
    let mut arguments = vec![
        "--reuse-window".to_owned(),
        project.to_string_lossy().into_owned(),
    ];
    if locations.iter().any(|(_, line)| line.is_some()) {
        arguments.push("--goto".to_owned());
    }
    arguments.extend(locations.iter().map(|(path, line)| match line {
        Some(line) => format!("{}:{}", path.display(), (*line).max(1)),
        None => path.to_string_lossy().into_owned(),
    }));
    launch(project, &arguments, None)
}

fn open_diff(project: &Path, path: &Path) -> Result<(), String> {
    let suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let mut base = tempfile::Builder::new()
        .prefix("farcaster-head-")
        .suffix(&suffix)
        .tempfile()
        .map_err(|error| format!("prepare VS Code diff: {error}"))?;
    base.write_all(&git_head_contents(path)?)
        .map_err(|error| format!("prepare VS Code diff: {error}"))?;
    let arguments = [
        "--wait".to_owned(),
        "--diff".to_owned(),
        base.path().to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned(),
    ];
    launch(project, &arguments, Some(base))
}

fn launch(
    project: &Path,
    arguments: &[String],
    temporary: Option<tempfile::NamedTempFile>,
) -> Result<(), String> {
    let mut child = Command::new("code")
        .args(arguments)
        .current_dir(project)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!("start VS Code (`code`): {error}. Install its shell command in PATH.")
        })?;
    std::thread::spawn(move || {
        match child.wait() {
            Ok(status) if !status.success() => {
                zlog::warn!("VS Code command exited with {status}");
            }
            Err(error) => zlog::warn!("VS Code command failed: {error}"),
            _ => {}
        }
        drop(temporary);
    });
    Ok(())
}
