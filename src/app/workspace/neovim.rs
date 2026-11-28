use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use gpui::{App, Context, Entity, IntoElement, Render, RenderImage, Task, Window};
use gpui_libghostty::TerminalOptions;
use rmpv::Value;

use super::Terminal;
use crate::{app::infrastructure::editor_launch, repository::git_head_contents};

#[path = "neovim_rpc.rs"]
mod rpc;
use rpc::NvimRpc;
pub(super) use rpc::ReviewSelection;

static NEXT_TAB: AtomicU64 = AtomicU64::new(1);
const SESSION_VIEW: &str = include_str!("neovim_session.lua");
const REVIEW_VIEW: &str = include_str!("neovim_review.lua");

#[derive(Debug, serde::Deserialize)]
pub(in crate::app) struct CodeContext {
    pub path: String,
    pub cursor_line: usize,
    pub cursor_column: usize,
    pub anchor_line: usize,
    pub anchor_column: usize,
    pub mode: String,
    pub text: String,
    pub modified: bool,
}

pub(in crate::app) enum EditorTarget {
    Resume,
    File(PathBuf, Option<u64>),
    Diff(PathBuf, Option<u64>),
    Transcript(String),
    Review(crate::reviews::Review),
    ReviewLocation {
        list_id: u64,
        index: usize,
        path: PathBuf,
    },
}

pub(super) fn new_session_tab() -> u64 {
    NEXT_TAB.fetch_add(1, Ordering::Relaxed)
}

pub(in crate::app) struct NvimEditor {
    terminal: Entity<Terminal>,
    session: Arc<EditorSession>,
    pending: Option<Task<()>>,
}

struct EditorSession {
    project: PathBuf,
    rpc: Mutex<NvimRpc>,
    _state_dir: tempfile::TempDir,
}

impl NvimEditor {
    pub(super) fn capture_code(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<CodeContext, String>> {
        self.request(cx, |rpc, _| capture_code(rpc))
    }

    pub(super) fn spawn<T: 'static>(
        project: PathBuf,
        selections: async_channel::Sender<ReviewSelection>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let executable = nvim_executable();
        let socket_dir = tempfile::Builder::new()
            .prefix("farcaster-neovim-")
            .tempdir()
            .map_err(|error| format!("create Neovim socket directory: {error}"))?;
        let socket = socket_dir.path().join("nvim.sock");
        let launch_file = socket_dir.path().join("launch.sh");
        let background_setup = socket_dir.path().join("background.lua");
        std::fs::write(&background_setup, include_str!("neovim_background.lua"))
            .map_err(|error| format!("prepare Neovim background sync: {error}"))?;
        let command = editor_launch::prepare(
            &launch_file,
            &executable,
            &[
                "-i".into(),
                socket_dir.path().join("shada").into_os_string(),
                "--cmd".into(),
                state_setup(socket_dir.path()).into(),
                "--listen".into(),
                socket.clone().into_os_string(),
                "-S".into(),
                background_setup.into_os_string(),
                "--".into(),
                project.clone().into_os_string(),
            ],
            &project,
        )?;
        let mut options = TerminalOptions::new(command, project.clone());
        options.quiet_login = true;
        let terminal = Terminal::spawn(options, window, cx)?;
        terminal.update(cx, |terminal, _| terminal.set_visible(false));
        Ok(Self {
            terminal,
            session: Arc::new(EditorSession {
                project,
                rpc: Mutex::new(NvimRpc::new(socket, selections)),
                _state_dir: socket_dir,
            }),
            pending: None,
        })
    }

    pub(super) fn is_alive(&self, cx: &App) -> bool {
        self.terminal.read(cx).is_alive()
    }

    pub(super) fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.focus(window, cx));
    }

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, _| terminal.set_visible(visible));
    }

    pub(super) fn snapshot(&mut self, cx: &mut Context<Self>) -> Result<Arc<RenderImage>, String> {
        self.terminal.update(cx, |terminal, _| terminal.snapshot())
    }

    pub(super) fn frame_count(&mut self, cx: &mut Context<Self>) -> u64 {
        self.terminal
            .update(cx, |terminal, _| terminal.frame_count())
    }

    pub(super) fn activate_tab(
        &mut self,
        tab: u64,
        target: EditorTarget,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<crate::reviews::ReviewNavigation>, String>> {
        self.request(cx, move |rpc, project| {
            open_target(rpc, project, tab, target)
        })
    }

    fn request<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        request: impl FnOnce(&mut NvimRpc, &Path) -> Result<T, String> + Send + 'static,
    ) -> Task<Result<T, String>> {
        let session = self.session.clone();
        let previous = self.pending.take();
        let (send, receive) = async_channel::bounded(1);
        self.pending = Some(cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = session
                .rpc
                .lock()
                .map_err(|_| "Neovim RPC state unavailable".to_owned())
                .and_then(|mut rpc| request(&mut rpc, &session.project));
            let _ = send.try_send(result);
        }));
        cx.background_executor().spawn(async move {
            receive
                .recv()
                .await
                .map_err(|_| "Neovim request cancelled".to_owned())?
        })
    }
}

impl Render for NvimEditor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.terminal.clone()
    }
}

pub(super) fn nvim_executable() -> PathBuf {
    farcaster_editors::EditorChoice::Neovim.program(Path::new("."), None)
}

fn state_setup(state_dir: &Path) -> String {
    let state = format!("{}//", state_dir.display());
    format!(
        "let g:auto_session_enabled = v:false | let &directory = {0} | let &backupdir = {0} | let &undodir = {0}",
        vim_string(&state)
    )
}

fn vim_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn activate_session(
    rpc: &mut NvimRpc,
    tab: u64,
    path: Option<&Path>,
    line: Option<u64>,
    scratch: Option<String>,
    diff: Option<Vec<u8>>,
    review: bool,
) -> Result<(), String> {
    rpc.exec(
        SESSION_VIEW,
        vec![
            tab.into(),
            path.map_or(Value::Nil, path_value),
            line.map_or(Value::Nil, |line| line.max(1).into()),
            scratch.map_or(Value::Nil, Value::from),
            diff.map_or(Value::Nil, Value::Binary),
            review.into(),
        ],
    )?;
    Ok(())
}

fn path_value(path: &Path) -> Value {
    path.to_string_lossy().as_ref().into()
}

fn open_target(
    rpc: &mut NvimRpc,
    project: &Path,
    tab: u64,
    target: EditorTarget,
) -> Result<Option<crate::reviews::ReviewNavigation>, String> {
    match target {
        EditorTarget::Resume => activate_session(rpc, tab, None, None, None, None, false)?,
        EditorTarget::File(path, line) => {
            activate_session(rpc, tab, Some(&path), line, None, None, false)?;
        }
        EditorTarget::Diff(path, line) => {
            let diff = git_head_contents(&path)?;
            activate_session(rpc, tab, Some(&path), line, None, Some(diff), false)?;
        }
        EditorTarget::Transcript(text) => {
            activate_session(rpc, tab, None, None, Some(text), None, false)?;
        }
        EditorTarget::Review(review) => {
            review.validate()?;
            let items = review
                .items
                .into_iter()
                .map(|location| {
                    let path = crate::reviews::resolve_path(project, &location.path)?;
                    Ok(Value::Map(vec![
                        ("path".into(), path_value(&path)),
                        (
                            "start_line".into(),
                            location.start_line.map_or(Value::Nil, Value::from),
                        ),
                        (
                            "end_line".into(),
                            location.end_line.map_or(Value::Nil, Value::from),
                        ),
                        ("note".into(), location.note.into()),
                    ]))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let payload = Value::Map(vec![
                ("title".into(), review.title.into()),
                ("items".into(), Value::Array(items)),
            ]);
            return open_review(rpc, tab, vec![payload]).map(Some);
        }
        EditorTarget::ReviewLocation {
            list_id,
            index,
            path,
        } => {
            return open_review(
                rpc,
                tab,
                vec![list_id.into(), (index + 1).into(), path_value(&path)],
            )
            .map(Some);
        }
    }
    Ok(None)
}

fn open_review(
    rpc: &mut NvimRpc,
    tab: u64,
    args: Vec<Value>,
) -> Result<crate::reviews::ReviewNavigation, String> {
    activate_session(rpc, tab, None, None, None, None, true)?;
    let output = rpc.exec(REVIEW_VIEW, args)?;
    rmpv::ext::from_value(output).map_err(|error| format!("Read review locations: {error}"))
}

fn capture_code(rpc: &mut NvimRpc) -> Result<CodeContext, String> {
    let output = rpc.exec(include_str!("neovim_capture.lua"), vec![])?;
    rmpv::ext::from_value(output).map_err(|error| format!("Read Neovim selection: {error}"))
}

#[cfg(test)]
#[path = "neovim_diff_tests.rs"]
mod diff_tests;

#[cfg(test)]
#[path = "neovim_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "neovim_tests.rs"]
mod tests;
