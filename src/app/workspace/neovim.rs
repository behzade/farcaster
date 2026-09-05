//! Project-scoped Neovim transport. Session views are native tabpages, not processes.

use std::{
    io::{Read as _, Seek as _},
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

pub(super) fn new_session_tab() -> u64 {
    NEXT_TAB.fetch_add(1, Ordering::Relaxed)
}

pub(in crate::app) struct NvimEditor {
    project: PathBuf,
    executable: PathBuf,
    socket_dir: Arc<tempfile::TempDir>,
    terminal: Entity<Terminal>,
    // Each request awaits its predecessor, including while the server starts.
    // Concurrent --remote-expr clients otherwise race to select different tabs.
    pending: Option<Task<()>>,
}

impl NvimEditor {
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
            "{} --listen {} -- {}",
            shell_quote(&executable),
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
        path: Option<PathBuf>,
        line: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let expression = session_expression(tab, path.as_deref(), line);
        let executable = self.executable.clone();
        let project = self.project.clone();
        let socket_dir = self.socket_dir.clone();
        let previous = self.pending.take();
        let (send, receive) = async_channel::bounded(1);
        self.pending = Some(cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = run_remote(
                &executable,
                &project,
                &socket_dir.path().join("nvim.sock"),
                &expression,
            );
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

fn vim_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn session_expression(tab: u64, path: Option<&Path>, line: Option<u64>) -> String {
    // Data stays in luaeval's argument list; neither filenames nor session keys
    // are interpolated into executable Lua or Ex commands.
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

fn run_remote(
    executable: &Path,
    project: &Path,
    socket: &Path,
    expression: &str,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        // A file avoids deadlocking on a full stderr pipe from user autocmds.
        let mut stderr = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut child = Command::new(executable)
            .current_dir(project)
            .args(["--server"])
            .arg(socket)
            .args(["--remote-expr", expression])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
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
            return Ok(());
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
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Neovim executable; runs a real headless server"]
    fn session_tabs_preserve_views_and_share_modified_buffers() -> Result<(), String> {
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let project = tempfile::tempdir().map_err(|error| error.to_string())?;
        let socket = project.path().join("nvim.sock");
        let executable = nvim_executable();
        let _server = Server(
            Command::new(&executable)
                .current_dir(project.path())
                .args(["--clean", "--headless", "-n", "-i", "NONE", "--listen"])
                .arg(&socket)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|error| error.to_string())?,
        );
        let a = project.path().join("a's | file.rs");
        let b = project.path().join("b.rs");
        for path in [&a, &b] {
            std::fs::write(path, "one\ntwo\nthree\nfour\nfive\n")
                .map_err(|error| error.to_string())?;
        }
        let request =
            |expression: String| run_remote(&executable, project.path(), &socket, &expression);
        let lua = |body: &str| {
            request(format!(
                "luaeval({})",
                vim_string(&format!("(function() {body}; return 0 end)()"))
            ))
        };
        request(session_expression(11, Some(&a), None))?;
        lua(r#"
            vim.o.hidden = false
            a_tab = vim.api.nvim_get_current_tabpage()
            a_buf = vim.api.nvim_get_current_buf()
            vim.cmd('vsplit')
            a_win = vim.api.nvim_get_current_win()
            vim.api.nvim_win_set_cursor(0, {4, 1})
            vim.api.nvim_buf_set_lines(0, 0, 1, false, {'unsaved'})
            "#)?;
        request(session_expression(22, Some(&b), Some(2)))?;
        lua(r#"
            b_tab = vim.api.nvim_get_current_tabpage()
            assert(b_tab ~= a_tab)
            assert(vim.api.nvim_win_get_cursor(0)[1] == 2)
            assert(vim.bo[a_buf].modified)
            assert(#vim.api.nvim_list_tabpages() == 2)
            "#)?;
        request(session_expression(11, None, None))?;
        lua(r#"
            assert(vim.api.nvim_get_current_tabpage() == a_tab)
            assert(vim.api.nvim_get_current_win() == a_win)
            assert(#vim.api.nvim_tabpage_list_wins(a_tab) == 2)
            assert(vim.deep_equal(vim.api.nvim_win_get_cursor(0), {4, 1}))
            assert(vim.api.nvim_get_current_buf() == a_buf)
            "#)?;
        // Opening the same file in another session shares its unsaved buffer,
        // but not the first session's cursor or split layout.
        request(session_expression(22, Some(&a), Some(1)))?;
        lua(r#"
            assert(vim.api.nvim_get_current_buf() == a_buf)
            assert(vim.api.nvim_get_current_line() == 'unsaved')
            assert(#vim.api.nvim_tabpage_list_wins(0) == 1)
            "#)?;
        request(session_expression(11, None, None))?;
        lua("assert(vim.deep_equal(vim.api.nvim_win_get_cursor(0), {4, 1}))")?;
        // A user-closed tab is recreated without invalid-handle errors.
        lua("vim.cmd('tabclose')")?;
        request(session_expression(11, Some(&b), None))?;
        lua(r#"
            assert(vim.api.nvim_get_current_tabpage() ~= b_tab)
            assert(#vim.api.nvim_list_tabpages() == 2)
            assert(vim.bo[a_buf].modified)
            "#)?;
        // The remote client must propagate Lua errors, not report success.
        assert!(lua("error('expected test error')").is_err());
        Ok(())
    }

    #[test]
    fn session_request_quotes_file_data_separately_from_lua() {
        let expression = session_expression(7, Some(Path::new("/tmp/it's | tricky.rs")), Some(42));
        assert!(expression.ends_with(", [7, '/tmp/it''s | tricky.rs', 42])"));
        assert!(session_expression(8, None, None).ends_with(", [8, v:null, v:null])"));
        assert_eq!(
            shell_quote(Path::new("/tmp/it's nvim")),
            "'/tmp/it'\\''s nvim'"
        );
    }
}
