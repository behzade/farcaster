//! Embedded Neovim process, PTY lifecycle, and Ghostty-backed GPUI terminal view.

use std::{
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, Styled as _, Task, Window, div,
};
use gpui_component::ElementExt as _;
use gpui_ghostty_terminal::{
    TerminalConfig, TerminalSession, default_terminal_font, default_terminal_font_features,
    view::{TerminalInput, TerminalView},
};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use wait_timeout::ChildExt as _;

const DEFAULT_COLS: u16 = 100;
const DEFAULT_ROWS: u16 = 30;
const OUTPUT_FRAME: Duration = Duration::from_millis(16);
static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct EditorTerminal {
    project: PathBuf,
    path: PathBuf,
    socket: PathBuf,
    nvim: PathBuf,
    view: Entity<TerminalView>,
    focus: FocusHandle,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    alive: Arc<AtomicBool>,
    size: PtySize,
    _output_task: Task<()>,
}

impl EditorTerminal {
    pub(crate) fn spawn<T: 'static>(
        project: PathBuf,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let nvim = std::env::var_os("PI_GUI_NVIM")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nvim"));
        Self::spawn_with_program(project, path, nvim, window, cx)
    }

    fn spawn_with_program<T: 'static>(
        project: PathBuf,
        path: PathBuf,
        nvim: PathBuf,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let socket = socket_path();
        let config = terminal_config();
        let session = TerminalSession::new(config)
            .map_err(|error| format!("initialize Ghostty terminal: {error}"))?;
        let size = PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|error| format!("open Neovim terminal: {error}"))?;
        let master = pair.master;
        let mut reader = master
            .try_clone_reader()
            .map_err(|error| format!("open Neovim terminal output: {error}"))?;
        let writer = master
            .take_writer()
            .map_err(|error| format!("open Neovim terminal input: {error}"))?;

        let mut command = CommandBuilder::new(&nvim);
        command.cwd(&project);
        command.arg("--listen");
        command.arg(&socket);
        command.arg("--");
        command.arg(&path);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "pi-gpui");
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("start {}: {error}", nvim.display()))?;
        let killer = child.clone_killer();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_after_exit = alive.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            alive_after_exit.store(false, Ordering::Release);
        });

        let writer = Arc::new(Mutex::new(writer));
        let input_writer = writer.clone();
        let focus = cx.focus_handle();
        let view = cx.new(|_| {
            TerminalView::new_with_input(
                session,
                focus.clone(),
                TerminalInput::new(move |bytes| {
                    let Ok(mut writer) = input_writer.lock() else {
                        return;
                    };
                    let _ = writer.write_all(bytes);
                    let _ = writer.flush();
                }),
            )
        });

        let (output_tx, output_rx) = async_channel::unbounded::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                let count = match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                if output_tx.send_blocking(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let output_view = view.clone();
        let output_task = cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(OUTPUT_FRAME).await;
                let mut batch = Vec::new();
                while let Ok(chunk) = output_rx.try_recv() {
                    batch.extend_from_slice(&chunk);
                }
                if !batch.is_empty() {
                    output_view.update(cx, |view, cx| view.queue_output_bytes(&batch, cx));
                }
                if output_rx.is_closed() && output_rx.is_empty() {
                    break;
                }
            }
        });

        focus.focus(window, cx);
        Ok(Self {
            project,
            path,
            socket,
            nvim,
            view,
            focus,
            master,
            killer,
            alive,
            size,
            _output_task: output_task,
        })
    }

    pub(crate) fn project(&self) -> &Path {
        &self.project
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(crate) fn focus<T>(&self, window: &mut Window, cx: &mut Context<T>) {
        self.focus.focus(window, cx);
    }

    pub(crate) fn open_file(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.is_alive() {
            return Err("the embedded Neovim process has exited".to_owned());
        }
        let mut remote = Command::new(&self.nvim)
            .current_dir(&self.project)
            .arg("--server")
            .arg(&self.socket)
            .arg("--remote")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("contact embedded Neovim: {error}"))?;
        let status = remote
            .wait_timeout(Duration::from_secs(1))
            .map_err(|error| format!("wait for embedded Neovim: {error}"))?;
        let Some(status) = status else {
            let _ = remote.kill();
            let _ = remote.wait();
            return Err("Neovim did not respond within one second".to_owned());
        };
        if !status.success() {
            return Err(format!("Neovim remote command exited with {status}"));
        }
        self.path = path;
        Ok(())
    }

    fn resize_to(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((cell_width, cell_height)) = cell_metrics(window) else {
            return;
        };
        let cols = grid_dimension(f32::from(bounds.size.width), cell_width);
        let rows = grid_dimension(f32::from(bounds.size.height), cell_height);
        if self.size.cols == cols && self.size.rows == rows {
            return;
        }
        let size = PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        };
        if self.master.resize(size).is_err() {
            return;
        }
        self.size = size;
        self.view
            .update(cx, |view, cx| view.resize_terminal(cols, rows, cx));
    }
}

impl Render for EditorTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = cx.entity().downgrade();
        div()
            .size_full()
            .min_h_0()
            .on_prepaint(move |bounds, window, cx| {
                let _ = editor.update(cx, |editor, cx| editor.resize_to(bounds, window, cx));
            })
            .child(self.view.clone())
    }
}

impl Drop for EditorTerminal {
    fn drop(&mut self) {
        let _ = self.killer.kill();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn terminal_config() -> TerminalConfig {
    let foreground = crate::theme::THEME.colors.text;
    let background = crate::theme::THEME.colors.canvas;
    TerminalConfig {
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
        default_fg: rgb(foreground),
        default_bg: rgb(background),
        update_window_title: false,
    }
}

fn rgb(color: gpui::Rgba) -> ghostty_vt::Rgb {
    ghostty_vt::Rgb {
        r: (color.r * 255.0).round() as u8,
        g: (color.g * 255.0).round() as u8,
        b: (color.b * 255.0).round() as u8,
    }
}

fn cell_metrics(window: &mut Window) -> Option<(f32, f32)> {
    let mut style = window.text_style();
    let font = default_terminal_font();
    style.font_family = font.family.clone();
    style.font_features = default_terminal_font_features();
    style.font_fallbacks = font.fallbacks.clone();
    let rem_size = window.rem_size();
    let font_size = style.font_size.to_pixels(rem_size);
    let line_height = style.line_height.to_pixels(style.font_size, rem_size);
    let line = window
        .text_system()
        .shape_text(
            SharedString::from("M"),
            font_size,
            &[style.to_run(1)],
            None,
            Some(1),
        )
        .ok()?
        .into_iter()
        .next()?;
    Some((
        f32::from(line.width()).max(1.0),
        f32::from(line_height).max(1.0),
    ))
}

fn grid_dimension(pixels: f32, cell: f32) -> u16 {
    (pixels / cell).floor().clamp(1.0, f32::from(u16::MAX)) as u16
}

fn socket_path() -> PathBuf {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pi-gpui-nvim-{}-{id}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_grid_never_has_zero_or_overflowing_dimensions() {
        assert_eq!(grid_dimension(0.0, 9.0), 1);
        assert_eq!(grid_dimension(89.0, 9.0), 9);
        assert_eq!(grid_dimension(f32::MAX, 1.0), u16::MAX);
    }

    #[test]
    fn editor_sockets_are_unique_within_the_process() {
        assert_ne!(socket_path(), socket_path());
    }

    #[cfg(unix)]
    #[gpui::test]
    fn embedded_editor_owns_a_live_pty_child(cx: &mut gpui::TestAppContext) {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("temporary project");
        let file = directory.path().join("file.rs");
        std::fs::write(&file, "fn main() {}\n").expect("test file");
        let program = directory.path().join("fake-nvim");
        std::fs::write(&program, "#!/bin/sh\nsleep 5\n").expect("fake Neovim");
        let mut permissions = std::fs::metadata(&program)
            .expect("fake Neovim metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).expect("executable fake Neovim");

        let cx = cx.add_empty_window();
        let editor = cx.update(|window, cx| {
            cx.new(|editor_cx| {
                EditorTerminal::spawn_with_program(
                    directory.path().to_path_buf(),
                    file,
                    program,
                    window,
                    editor_cx,
                )
            })
        });
        let editor = editor.read_with(cx, |editor, _| {
            editor.as_ref().expect("embedded editor starts").is_alive()
        });
        assert!(editor);
    }
}
