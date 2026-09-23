use std::path::{Path, PathBuf};

use gpui::{Context, Window};

use super::{
    FarcasterApp,
    editor::{EditorBackend, EditorRequest},
    external_editor,
};

pub(super) struct ZedBackend;

impl EditorBackend for ZedBackend {
    fn name(&self) -> &'static str {
        "Zed"
    }

    fn program(&self, project: &Path) -> PathBuf {
        farcaster_editors::EditorChoice::Zed.program(project, std::env::var_os("PATH").as_deref())
    }

    fn open(
        &self,
        _app: &mut FarcasterApp,
        request: EditorRequest,
        _window: &mut Window,
        _cx: &mut Context<FarcasterApp>,
    ) -> Result<(), String> {
        let project = match &request {
            EditorRequest::Project(project)
            | EditorRequest::File { project, .. }
            | EditorRequest::Review { project, .. } => project,
        };
        let program = self.program(project);
        match request {
            EditorRequest::Project(project) => external_editor::launch(
                &program,
                "Zed",
                &project,
                &[project.to_string_lossy().into_owned()],
                None,
            ),
            EditorRequest::File {
                project,
                path,
                line,
                diff,
            } => {
                if diff {
                    let base = external_editor::head_tempfile(&path, "Zed")?;
                    let args = [
                        "--wait".into(),
                        "--diff".into(),
                        base.path().to_string_lossy().into_owned(),
                        path.to_string_lossy().into_owned(),
                    ];
                    external_editor::launch(&program, "Zed", &project, &args, Some(base))
                } else {
                    external_editor::launch(
                        &program,
                        "Zed",
                        &project,
                        &[
                            project.to_string_lossy().into_owned(),
                            location(&path, line),
                        ],
                        None,
                    )
                }
            }
            EditorRequest::Review {
                project, locations, ..
            } => {
                let mut args = vec![project.to_string_lossy().into_owned()];
                args.extend(locations.iter().map(|(path, line)| location(path, *line)));
                external_editor::launch(&program, "Zed", &project, &args, None)
            }
        }
    }
}

fn location(path: &Path, line: Option<u64>) -> String {
    match line {
        Some(line) => format!("{}:{}", path.display(), line.max(1)),
        None => path.to_string_lossy().into_owned(),
    }
}
