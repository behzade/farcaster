//! Embedded Neovim component for GPUI, rendered by [`gpui_ghostty`].

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use gpui::{App, Context, Entity, IntoElement, Render, Window};
use gpui_ghostty::{Terminal, TerminalOptions};
use wait_timeout::ChildExt as _;

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);
const DEFAULT_REMOTE_TIMEOUT: Duration = Duration::from_secs(1);

/// Configuration for an embedded Neovim instance.
pub struct NvimOptions {
    pub project: PathBuf,
    pub initial_file: PathBuf,
    pub initial_line: Option<u64>,
    pub executable: PathBuf,
    pub remote_timeout: Duration,
}

impl NvimOptions {
    pub fn new(project: impl Into<PathBuf>, initial_file: impl Into<PathBuf>) -> Self {
        Self {
            project: project.into(),
            initial_file: initial_file.into(),
            initial_line: None,
            executable: std::env::var_os("GPUI_NVIM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("nvim")),
            remote_timeout: DEFAULT_REMOTE_TIMEOUT,
        }
    }
}

/// A Neovim process hosted inside a [`Terminal`].
pub struct NvimEditor {
    project: PathBuf,
    socket: PathBuf,
    executable: PathBuf,
    remote_timeout: Duration,
    terminal: Entity<Terminal>,
}

impl NvimEditor {
    pub fn spawn<T: 'static>(
        options: NvimOptions,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let socket = socket_path();
        let command = nvim_command(
            &options.executable,
            &socket,
            &options.initial_file,
            options.initial_line,
        );
        let terminal = Terminal::spawn(
            TerminalOptions::new(command, options.project.clone()),
            window,
            cx,
        )?;
        Ok(Self {
            project: options.project,
            socket,
            executable: options.executable,
            remote_timeout: options.remote_timeout,
            terminal,
        })
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn is_alive(&self, cx: &App) -> bool {
        self.terminal.read(cx).is_alive()
    }

    pub fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.focus(window, cx));
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, _| terminal.set_visible(visible));
    }

    /// Opens `path` and places the cursor at `line` when supplied.
    pub fn open_file_at_line(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.terminal.read(cx).is_alive() {
            return Err("the embedded Neovim process has exited".to_owned());
        }
        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.project)
            .arg("--server")
            .arg(&self.socket)
            .arg("--remote");
        if let Some(line) = line {
            command.arg(format!("+{line}"));
        }
        let mut remote = command
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("contact embedded Neovim: {error}"))?;
        let status = remote
            .wait_timeout(self.remote_timeout)
            .map_err(|error| format!("wait for embedded Neovim: {error}"))?;
        let Some(status) = status else {
            let _ = remote.kill();
            let _ = remote.wait();
            return Err(format!(
                "Neovim did not respond within {:?}",
                self.remote_timeout
            ));
        };
        if !status.success() {
            return Err(format!("Neovim remote command exited with {status}"));
        }
        Ok(())
    }
}

impl Render for NvimEditor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.terminal.clone()
    }
}

fn nvim_command(executable: &Path, socket: &Path, path: &Path, line: Option<u64>) -> String {
    let line = line.map_or_else(String::new, |line| format!(" +{line}"));
    format!(
        "{} --listen {}{} -- {}",
        shell_quote(executable),
        shell_quote(socket),
        line,
        shell_quote(path)
    )
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn socket_path() -> PathBuf {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("gpui-neovim-{}-{id}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_paths_at_the_ghostty_boundary() {
        assert_eq!(
            nvim_command(
                Path::new("/tmp/my nvim"),
                Path::new("/tmp/editor.sock"),
                Path::new("/tmp/it's.rs"),
                Some(42),
            ),
            "'/tmp/my nvim' --listen '/tmp/editor.sock' +42 -- '/tmp/it'\\''s.rs'"
        );
    }
}
