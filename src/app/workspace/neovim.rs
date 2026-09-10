use std::{
    io::{Read as _, Seek as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{App, Context, Entity, IntoElement, Render, RenderImage, Task, Window};
use gpui_libghostty::{Terminal, TerminalOptions};

static NEXT_TAB: AtomicU64 = AtomicU64::new(1);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_INTERVAL: Duration = Duration::from_millis(25);
const SESSION_VIEW: &str = include_str!("neovim_session.lua");

#[path = "neovim_diff.rs"]
mod diff;
use diff::head_contents;

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

pub(super) enum EditorTarget {
    Resume,
    File(PathBuf, Option<u64>),
    Diff(PathBuf, Option<u64>),
    Transcript(String),
    Review(crate::app::reviews::Review),
}

pub(super) fn new_session_tab() -> u64 {
    NEXT_TAB.fetch_add(1, Ordering::Relaxed)
}

pub(in crate::app) struct NvimEditor {
    project: PathBuf,
    executable: PathBuf,
    socket_dir: Arc<tempfile::TempDir>,
    terminal: Entity<Terminal>,
    pending: Option<Task<()>>,
}

impl NvimEditor {
    pub(super) fn capture_code(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<CodeContext, String>> {
        self.request(cx, |executable, project, state_dir| {
            capture_code(executable, project, &state_dir.join("nvim.sock"))
        })
    }

    pub(super) fn spawn<T: 'static>(
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let executable = nvim_executable();
        let socket_dir = Arc::new(
            tempfile::Builder::new()
                .prefix("farcaster-neovim-")
                .tempdir()
                .map_err(|error| format!("create Neovim socket directory: {error}"))?,
        );
        let command = format!(
            "{} -i {} --cmd {} --listen {} -- {}",
            shell_quote(&executable),
            shell_quote(&socket_dir.path().join("shada")),
            shell_quote(Path::new(&state_setup(socket_dir.path()))),
            shell_quote(&socket_dir.path().join("nvim.sock")),
            shell_quote(&project),
        );
        let terminal = Terminal::spawn(TerminalOptions::new(command, project.clone()), window, cx)?;
        terminal.update(cx, |terminal, _| terminal.set_visible(false));
        Ok(Self {
            project,
            executable,
            socket_dir,
            terminal,
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

    pub(super) fn activate_tab(
        &mut self,
        tab: u64,
        target: EditorTarget,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.request(cx, move |executable, project, state_dir| {
            open_target(executable, project, state_dir, tab, target)
        })
    }

    fn request<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        request: impl FnOnce(&Path, &Path, &Path) -> Result<T, String> + Send + 'static,
    ) -> Task<Result<T, String>> {
        let executable = self.executable.clone();
        let project = self.project.clone();
        let socket_dir = self.socket_dir.clone();
        let previous = self.pending.take();
        let (send, receive) = async_channel::bounded(1);
        self.pending = Some(cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = request(&executable, &project, socket_dir.path());
            let _ = send.send(result).await;
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

fn nvim_executable() -> PathBuf {
    std::env::var_os("FARCASTER_NVIM")
        .or_else(|| std::env::var_os("GPUI_NVIM"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("nvim"))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn state_setup(state_dir: &Path) -> String {
    let state = format!("{}//", state_dir.display());
    format!(
        "let &directory = {0} | let &backupdir = {0} | let &undodir = {0}",
        vim_string(&state)
    )
}

fn vim_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn session_expression(tab: u64, path: Option<&Path>, line: Option<u64>) -> String {
    let path = path.map_or_else(
        || "v:null".to_owned(),
        |path| vim_string(&path.to_string_lossy()),
    );
    let line = line.map_or_else(|| "v:null".to_owned(), |line| line.max(1).to_string());
    format!(
        "luaeval({}, [{tab}, {path}, {line}])",
        vim_string(SESSION_VIEW)
    )
}

fn scratch_expression(tab: u64, path: &Path) -> String {
    format!(
        "luaeval({}, [{tab}, v:null, v:null, {}])",
        vim_string(SESSION_VIEW),
        vim_string(&path.to_string_lossy()),
    )
}

fn open_target(
    executable: &Path,
    project: &Path,
    state_dir: &Path,
    tab: u64,
    target: EditorTarget,
) -> Result<(), String> {
    // Keep large transcripts out of the command line. Hold the transfer file
    // until Neovim has read it, then let it drop.
    let (expression, _transfer) = match target {
        EditorTarget::Resume => (session_expression(tab, None, None), None),
        EditorTarget::File(path, line) => (session_expression(tab, Some(&path), line), None),
        EditorTarget::Diff(path, line) => {
            let mut file =
                tempfile::NamedTempFile::new_in(state_dir).map_err(|error| error.to_string())?;
            file.write_all(&head_contents(&path)?)
                .map_err(|error| error.to_string())?;
            let expression = format!(
                "luaeval({}, [{tab}, {}, {}, v:null, {}])",
                vim_string(SESSION_VIEW),
                vim_string(&path.to_string_lossy()),
                line.map_or_else(|| "v:null".to_owned(), |line| line.max(1).to_string()),
                vim_string(&file.path().to_string_lossy()),
            );
            (expression, Some(file))
        }
        EditorTarget::Review(review) => {
            review.validate()?;
            let items = review
                .items
                .iter()
                .map(|location| {
                    let path = crate::app::reviews::resolve_path(project, &location.path)?;
                    Ok(serde_json::json!({
                        "path": path,
                        "start_line": location.start_line,
                        "end_line": location.end_line,
                        "note": location.note,
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let payload = serde_json::json!({ "title": review.title, "items": items });
            run_remote(
                executable,
                project,
                &state_dir.join("nvim.sock"),
                &session_expression(tab, None, None),
            )?;
            let mut file = tempfile::NamedTempFile::new_in(state_dir)
                .map_err(|error| error.to_string())?;
            serde_json::to_writer(&mut file, &payload).map_err(|error| error.to_string())?;
            let expression = format!(
                "luaeval({}, {})",
                vim_string(include_str!("neovim_review.lua")),
                vim_string(&file.path().to_string_lossy()),
            );
            (expression, Some(file))
        }
        EditorTarget::Transcript(text) => {
            let mut file =
                tempfile::NamedTempFile::new_in(state_dir).map_err(|error| error.to_string())?;
            file.write_all(text.as_bytes())
                .map_err(|error| error.to_string())?;
            (scratch_expression(tab, file.path()), Some(file))
        }
    };
    run_remote(
        executable,
        project,
        &state_dir.join("nvim.sock"),
        &expression,
    )
}

fn run_remote(
    executable: &Path,
    project: &Path,
    socket: &Path,
    expression: &str,
) -> Result<(), String> {
    remote_output(executable, project, socket, expression).map(|_| ())
}

fn capture_code(executable: &Path, project: &Path, socket: &Path) -> Result<CodeContext, String> {
    let expression = format!(
        "luaeval({})",
        vim_string(include_str!("neovim_capture.lua"))
    );
    let output = remote_output(executable, project, socket, &expression)?;
    serde_json::from_str(&output).map_err(|error| format!("Read Neovim selection: {error}"))
}

fn remote_output(
    executable: &Path,
    project: &Path,
    socket: &Path,
    expression: &str,
) -> Result<String, String> {
    let started = Instant::now();
    loop {
        let mut stderr = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut stdout = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut child = Command::new(executable)
            .current_dir(project)
            .args(["--server"])
            .arg(socket)
            .args(["--remote-expr", expression])
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().map_err(|error| error.to_string())?)
            .stderr(stderr.try_clone().map_err(|error| error.to_string())?)
            .spawn()
            .map_err(|error| format!("contact embedded Neovim: {error}"))?;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < REMOTE_TIMEOUT => {
                    std::thread::sleep(RETRY_INTERVAL)
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(match result {
                        Err(error) => format!("wait for Neovim: {error}"),
                        _ => "Neovim remote request timed out".to_owned(),
                    });
                }
            }
        };
        if status.success() {
            stdout.rewind().map_err(|error| error.to_string())?;
            let mut output = String::new();
            stdout
                .take(2 * 1024 * 1024)
                .read_to_string(&mut output)
                .map_err(|error| format!("Read Neovim response: {error}"))?;
            return Ok(output);
        }
        let mut detail = String::new();
        let _ = stderr.rewind();
        let _ = stderr.take(8192).read_to_string(&mut detail);
        if started.elapsed() >= REMOTE_TIMEOUT
            || !(detail.contains("E247:") || detail.contains("Failed to connect"))
        {
            return Err(format!(
                "Neovim remote request failed: {status}: {}",
                detail.trim()
            ));
        }
        std::thread::sleep(RETRY_INTERVAL);
    }
}

#[cfg(test)]
#[path = "neovim_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "neovim_tests.rs"]
mod tests;
