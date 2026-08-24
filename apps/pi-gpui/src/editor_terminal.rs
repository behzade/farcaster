//! Embedded Neovim lifecycle and upstream libghostty native surface integration.

use std::{
    ffi::{CString, c_void},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use gpui::{
    Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent,
    Styled as _, Task, Window, div,
};
use gpui_component::ElementExt as _;
use pi_libghostty::{KeyAction, Modifiers, MouseButton, MouseState, Surface};
use raw_window_handle::RawWindowHandle;
use wait_timeout::ChildExt as _;

const TICK_INTERVAL: Duration = Duration::from_millis(8);
static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct EditorTerminal {
    project: PathBuf,
    path: PathBuf,
    socket: PathBuf,
    nvim: PathBuf,
    surface: Surface,
    focus: FocusHandle,
    bounds: Bounds<Pixels>,
    tick_task: Option<Task<()>>,
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
        let socket = socket_path();
        let command = nvim_command(&nvim, &socket, &path)?;
        let working_directory = path_c_string(&project, "editor project")?;
        let parent_view = appkit_view(window)?;
        let surface = Surface::new(parent_view, &working_directory, &command)
            .map_err(|error| format!("initialize upstream libghostty: {error}"))?;
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Ok(Self {
            project,
            path,
            socket,
            nvim,
            surface,
            focus,
            bounds: Bounds::default(),
            tick_task: None,
        })
    }

    pub(crate) fn project(&self) -> &Path {
        &self.project
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.surface.is_alive()
    }

    pub(crate) fn focus<T>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        self.surface.set_visible(true);
        self.surface.set_focus(true);
        self.focus.focus(window, cx);
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.surface.set_visible(visible);
        self.surface.set_focus(visible);
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

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        if self.tick_task.is_some() {
            return;
        }
        self.surface.tick();
        let editor = cx.entity().downgrade();
        self.tick_task = Some(cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                let updated = editor.update(cx, |editor, cx| {
                    if editor.surface.needs_tick() {
                        editor.surface.tick();
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    fn update_frame(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        self.surface.set_frame(
            f64::from(f32::from(bounds.origin.x)),
            f64::from(f32::from(bounds.origin.y)),
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        );
        self.surface.set_visible(true);
    }

    fn key_down(&mut self, event: &KeyDownEvent) {
        self.send_key(
            if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
            &event.keystroke,
        );
    }

    fn key_up(&mut self, event: &KeyUpEvent) {
        self.send_key(KeyAction::Release, &event.keystroke);
    }

    fn send_key(&mut self, action: KeyAction, keystroke: &gpui::Keystroke) {
        let Some(keycode) = mac_keycode(&keystroke.key) else {
            if matches!(action, KeyAction::Press | KeyAction::Repeat)
                && !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
                && let Some(text) = keystroke.key_char.as_deref()
                && let Ok(text) = CString::new(text)
            {
                self.surface.text(&text);
            }
            return;
        };
        let text = keystroke
            .key_char
            .as_deref()
            .and_then(|text| CString::new(text).ok());
        let unshifted = keystroke.key.chars().next().map_or(0, u32::from);
        let _ = self.surface.key(
            action,
            modifiers(keystroke.modifiers),
            keycode,
            text.as_deref(),
            unshifted,
        );
    }

    fn mouse_position(&mut self, position: gpui::Point<Pixels>, modifiers: gpui::Modifiers) {
        let x = f64::from(f32::from(position.x - self.bounds.origin.x));
        let y = f64::from(f32::from(position.y - self.bounds.origin.y));
        self.surface
            .mouse_position(x, y, self::modifiers(modifiers));
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.surface.set_focus(true);
        self.mouse_position(event.position, event.modifiers);
        self.surface.mouse_button(
            MouseState::Press,
            mouse_button(event.button),
            modifiers(event.modifiers),
        );
    }

    fn mouse_up(&mut self, event: &MouseUpEvent) {
        self.mouse_position(event.position, event.modifiers);
        self.surface.mouse_button(
            MouseState::Release,
            mouse_button(event.button),
            modifiers(event.modifiers),
        );
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent) {
        self.mouse_position(event.position, event.modifiers);
    }

    fn scroll(&mut self, event: &ScrollWheelEvent) {
        self.mouse_position(event.position, event.modifiers);
        let (x, y, precision) = match event.delta {
            ScrollDelta::Pixels(delta) => (
                f64::from(f32::from(delta.x)),
                f64::from(f32::from(delta.y)),
                true,
            ),
            ScrollDelta::Lines(delta) => (f64::from(delta.x), f64::from(delta.y), false),
        };
        self.surface.mouse_scroll(x, y, precision);
    }
}

impl Render for EditorTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_ticking(cx);
        let editor = cx.entity().downgrade();
        div()
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .on_prepaint(move |bounds, _, cx| {
                let _ = editor.update(cx, |editor, _| editor.update_frame(bounds));
            })
            .on_key_down(cx.listener(|editor, event, _, _| editor.key_down(event)))
            .on_key_up(cx.listener(|editor, event, _, _| editor.key_up(event)))
            .on_mouse_move(cx.listener(|editor, event, _, _| editor.mouse_move(event)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|editor, event, window, cx| editor.mouse_down(event, window, cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Middle,
                cx.listener(|editor, event, window, cx| editor.mouse_down(event, window, cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|editor, event, window, cx| editor.mouse_down(event, window, cx)),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|editor, event, _, _| editor.mouse_up(event)),
            )
            .on_mouse_up(
                gpui::MouseButton::Middle,
                cx.listener(|editor, event, _, _| editor.mouse_up(event)),
            )
            .on_mouse_up(
                gpui::MouseButton::Right,
                cx.listener(|editor, event, _, _| editor.mouse_up(event)),
            )
            .on_scroll_wheel(cx.listener(|editor, event, _, _| editor.scroll(event)))
    }
}

fn appkit_view(window: &Window) -> Result<NonNull<c_void>, String> {
    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .map_err(|error| format!("read native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view),
        _ => Err("upstream libghostty editor is only available on macOS".to_owned()),
    }
}

fn path_c_string(path: &Path, label: &str) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| format!("{label} contains a NUL byte: {}", path.display()))
}

fn nvim_command(nvim: &Path, socket: &Path, path: &Path) -> Result<CString, String> {
    let command = format!(
        "{} --listen {} -- {}",
        shell_quote(nvim),
        shell_quote(socket),
        shell_quote(path)
    );
    CString::new(command).map_err(|_| "Neovim command contains a NUL byte".to_owned())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn socket_path() -> PathBuf {
    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pi-gpui-nvim-{}-{id}.sock", std::process::id()))
}

fn modifiers(value: gpui::Modifiers) -> Modifiers {
    let mut result = Modifiers::empty();
    if value.shift {
        result.insert(Modifiers::SHIFT);
    }
    if value.control {
        result.insert(Modifiers::CONTROL);
    }
    if value.alt {
        result.insert(Modifiers::ALT);
    }
    if value.platform {
        result.insert(Modifiers::SUPER);
    }
    result
}

fn mouse_button(button: gpui::MouseButton) -> MouseButton {
    match button {
        gpui::MouseButton::Left => MouseButton::Left,
        gpui::MouseButton::Right => MouseButton::Right,
        gpui::MouseButton::Middle => MouseButton::Middle,
        gpui::MouseButton::Navigate(_) => MouseButton::Unknown,
    }
}

fn mac_keycode(key: &str) -> Option<u32> {
    Some(match key {
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "=" => 24,
        "9" => 25,
        "7" => 26,
        "-" => 27,
        "8" => 28,
        "0" => 29,
        "]" => 30,
        "o" => 31,
        "u" => 32,
        "[" => 33,
        "i" => 34,
        "p" => 35,
        "enter" | "return" => 36,
        "l" => 37,
        "j" => 38,
        "'" => 39,
        "k" => 40,
        ";" => 41,
        "\\" => 42,
        "," => 43,
        "/" => 44,
        "n" => 45,
        "m" => 46,
        "." => 47,
        "tab" => 48,
        "space" => 49,
        "`" => 50,
        "backspace" => 51,
        "escape" => 53,
        "f17" => 64,
        "f18" => 79,
        "f19" => 80,
        "f20" => 90,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f3" => 99,
        "f8" => 100,
        "f9" => 101,
        "f11" => 103,
        "f13" => 105,
        "f16" => 106,
        "f14" => 107,
        "f10" => 109,
        "f12" => 111,
        "f15" => 113,
        "home" => 115,
        "pageup" | "page_up" | "page-up" => 116,
        "delete" => 117,
        "f4" => 118,
        "end" => 119,
        "f2" => 120,
        "pagedown" | "page_down" | "page-down" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvim_command_quotes_paths_for_ghosttys_shell_boundary() {
        let command = nvim_command(
            Path::new("/tmp/my nvim"),
            Path::new("/tmp/editor.sock"),
            Path::new("/tmp/it's.rs"),
        )
        .expect("valid command");
        assert_eq!(
            command.to_str().expect("UTF-8 command"),
            "'/tmp/my nvim' --listen '/tmp/editor.sock' -- '/tmp/it'\\''s.rs'"
        );
    }

    #[test]
    fn editor_sockets_are_unique_within_the_process() {
        assert_ne!(socket_path(), socket_path());
    }

    #[test]
    fn keycode_mapping_covers_neovim_navigation_and_repeat_keys() {
        for key in ["j", "k", "up", "down", "pageup", "pagedown", "escape"] {
            assert!(mac_keycode(key).is_some(), "missing keycode for {key}");
        }
    }
}
