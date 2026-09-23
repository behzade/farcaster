use gpui::{Context, Window};
use std::path::{Path, PathBuf};

use super::{
    FarcasterApp,
    editor::{EditorBackend, EditorRequest},
    external_editor,
};

pub(super) struct VsCodeBackend;

impl EditorBackend for VsCodeBackend {
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
    arguments.extend(
        locations
            .iter()
            .map(|(path, line)| external_editor::location(path, *line)),
    );
    launch(project, &arguments, None)
}

fn open_diff(project: &Path, path: &Path) -> Result<(), String> {
    let base = external_editor::head_tempfile(path, "VS Code")?;
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
    external_editor::launch(Path::new("code"), "VS Code", project, arguments, temporary)
}
