use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{Context, KeyUpEvent, Window};

use super::{neovim::CodeContext, send_to_chat::CodeDestination};
use crate::app::{AppSurface, FarcasterApp, ui::navigation::is_prefix_chord};

#[path = "voice_hex.rs"]
mod hex;
#[path = "voice_speech.rs"]
mod speech;

const HOLD_DELAY: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(in crate::app) struct VoiceState {
    pub(in crate::app) available: bool,
    pub(in crate::app) settings_error: Option<String>,
    availability_check: Option<gpui::Task<()>>,
    held: bool,
    press: u64,
    input: Option<Input>,
    pending: HashMap<String, String>,
    replies: HashSet<PathBuf>,
    speech: Option<speech::Speech>,
    speech_generation: u64,
}

struct Input {
    id: String,
    destination: CodeDestination,
    project: PathBuf,
    context: Option<CodeContext>,
    capturing_context: bool,
    transcript: Option<String>,
    recorder: hex::Dictation,
    recording: bool,
}

impl VoiceState {
    pub(in crate::app) fn recording(&self) -> bool {
        self.held && self.input.as_ref().is_some_and(|input| input.recording)
    }

    pub(in crate::app) fn submission_result(
        &mut self,
        id: Option<&str>,
        accepted: bool,
        session: Option<&Path>,
    ) {
        if id.and_then(|id| self.pending.remove(id)).is_some()
            && accepted
            && let Some(session) = session
        {
            self.replies.insert(session.to_path_buf());
        }
    }

    pub(in crate::app) fn stopped(&mut self, target: &str, session: Option<&Path>) {
        self.pending.retain(|_, destination| destination != target);
        if let Some(session) = session {
            self.replies.remove(session);
        }
    }
}

impl FarcasterApp {
    pub(in crate::app) fn refresh_voice_availability(&mut self, cx: &mut Context<Self>) {
        self.workspace.voice.availability_check = Some(cx.spawn(async move |weak, cx| {
            let available = cx
                .background_executor()
                .spawn(async { hex::available() })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.workspace.voice.available = available;
                cx.notify();
            });
        }));
    }

    pub(in crate::app) fn voice_enabled(&self) -> bool {
        self.settings.voice_enabled && self.workspace.voice.available
    }

    pub(in crate::app) fn toggle_settings_voice(&mut self, cx: &mut Context<Self>) {
        if !self.workspace.voice.available {
            return;
        }
        let enabled = !self.settings.voice_enabled;
        match crate::app::persistence::open().and_then(|store| store.save_voice_enabled(enabled)) {
            Ok(()) => {
                self.settings.voice_enabled = enabled;
                self.workspace.voice.settings_error = None;
                if !enabled {
                    self.cancel_voice(cx);
                    self.workspace.voice.pending.clear();
                    self.workspace.voice.replies.clear();
                }
            }
            Err(error) => self.workspace.voice.settings_error = Some(error),
        }
        cx.notify();
    }

    /// Observe the same key as app navigation. A tap remains a navigation prefix.
    pub(in crate::app) fn voice_key_down(
        &mut self,
        key: &str,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if key == "escape"
            && (self.workspace.voice.input.is_some() || self.workspace.voice.speech.is_some())
        {
            let held = self.workspace.voice.held;
            self.cancel_voice(cx);
            // G may still be physically down: suppress its repeats until key-up.
            self.workspace.voice.held = held;
            return true;
        }
        if !cfg!(target_os = "macos") || !self.voice_enabled() {
            return false;
        }
        if !is_prefix_chord(key, modifiers) {
            if self.workspace.voice.input.is_none() {
                self.workspace.voice.held = false;
            }
            return false;
        }
        if self.workspace.voice.held {
            return true;
        }
        if !matches!(
            self.workspace.surface,
            AppSurface::Chat | AppSurface::Editor
        ) || self.native_workspace_covered_by_overlay()
        {
            return false;
        }
        if self.workspace.voice.speech.take().is_some() {
            cx.notify();
        }
        self.workspace.voice.held = true;
        self.workspace.voice.press = self.workspace.voice.press.wrapping_add(1);
        let press = self.workspace.voice.press;
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor().timer(HOLD_DELAY).await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.workspace.voice.held && this.workspace.voice.press == press {
                    this.navigation.chat.activation.clear();
                    this.notify_composer(cx);
                    this.start_voice(window, cx);
                }
            });
        })
        .detach();
        false
    }

    pub(in crate::app) fn voice_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "g" || !self.workspace.voice.held {
            return;
        }
        self.workspace.voice.held = false;
        if let Some(input) = self.workspace.voice.input.as_mut() {
            input.recorder.finish();
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(in crate::app) fn cancel_voice(&mut self, cx: &mut Context<Self>) {
        self.workspace.voice.held = false;
        self.workspace.voice.input.take();
        self.workspace.voice.speech.take();
        cx.notify();
    }

    fn start_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.voice.speech.take();
        // A second recording can replace a transcription that has not been sent yet.
        self.workspace.voice.input.take();
        if !self.voice_enabled()
            || !matches!(
                self.workspace.surface,
                AppSurface::Chat | AppSurface::Editor
            )
            || self.native_workspace_covered_by_overlay()
        {
            return;
        }
        let editor = if self.workspace.surface == AppSurface::Editor {
            let Some(editor) = self
                .workspace
                .editor
                .view
                .clone()
                .filter(|_| self.workspace.editor.ready)
            else {
                return;
            };
            Some(editor)
        } else {
            None
        };
        let project = self.workspace_project();
        let harness = self.active_harness().to_owned();
        if harness.is_some_and(|harness| !self.ensure_backend_trust(harness, &project, window, cx))
        {
            return;
        }
        let (recorder, events) = match hex::Dictation::start() {
            Ok(recording) => recording,
            Err(error) => {
                self.notify_workspace_error("Voice", error, cx);
                return;
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        let capture = editor.map(|editor| editor.update(cx, |editor, cx| editor.capture_code(cx)));
        self.workspace.voice.input = Some(Input {
            id: id.clone(),
            destination: CodeDestination {
                target: self.composer.sessions.current_target().to_owned(),
                session: self.snapshot.session_target(),
                label: "Current chat".into(),
                harness,
            },
            project,
            context: None,
            capturing_context: capture.is_some(),
            transcript: None,
            recorder,
            recording: false,
        });
        if let Some(capture) = capture {
            let capture_id = id.clone();
            cx.spawn_in(window, async move |weak, cx| {
                let result = capture.await;
                let _ = weak.update_in(cx, |this, window, cx| {
                    let Some(input) = this
                        .workspace
                        .voice
                        .input
                        .as_mut()
                        .filter(|input| input.id == capture_id)
                    else {
                        return;
                    };
                    match result {
                        Ok(context) => {
                            input.context = Some(context);
                            input.capturing_context = false;
                        }
                        Err(error) => {
                            this.voice_failed(error, cx);
                            return;
                        }
                    }
                    this.submit_voice_if_ready(window, cx);
                });
            })
            .detach();
        }
        cx.spawn_in(window, async move |weak, cx| {
            while let Ok(event) = events.recv().await {
                let keep = weak
                    .update_in(cx, |this, window, cx| {
                        let Some(input) = this
                            .workspace
                            .voice
                            .input
                            .as_mut()
                            .filter(|input| input.id == id)
                        else {
                            return false;
                        };
                        match event {
                            hex::Event::Recording => input.recording = true,
                            hex::Event::Transcript(text) => input.transcript = Some(text),
                            hex::Event::Failed(error) => {
                                this.voice_failed(error, cx);
                                return false;
                            }
                        }
                        this.submit_voice_if_ready(window, cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn voice_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.workspace.voice.input.take();
        self.notify_workspace_error("Voice", error, cx);
    }

    fn submit_voice_if_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .workspace
            .voice
            .input
            .as_ref()
            .is_some_and(|input| !input.capturing_context && input.transcript.is_some())
        {
            return;
        }
        let Some(input) = self.workspace.voice.input.take() else {
            return;
        };
        let Some(transcript) = input.transcript else {
            return;
        };
        if transcript.trim().is_empty() {
            cx.notify();
            return;
        }
        let target = input.destination.target.clone();
        let message = match input.context {
            Some(context) => context.prompt(&transcript),
            None => transcript.trim().to_owned(),
        };
        if let Some(id) = self.submit_to_chat(input.destination, input.project, message, window, cx)
        {
            self.dismiss_code_task_notice(cx);
            self.workspace.voice.pending.insert(id, target);
        }
        cx.notify();
    }

    pub(in crate::app) fn speak_voice_reply(
        &mut self,
        body: &str,
        target: Option<&(PathBuf, PathBuf)>,
        cx: &mut Context<Self>,
    ) {
        let Some((session, _)) = target else {
            return;
        };
        if !self.workspace.voice.replies.remove(session) {
            return;
        }
        if self.workspace.surface == AppSurface::Chat || self.workspace.voice.input.is_some() {
            cx.notify();
            return;
        }
        let Some(text) = speech::spoken_opening(body) else {
            cx.notify();
            return;
        };
        self.workspace.voice.speech.take();
        let (player, done) = match speech::Speech::start(text) {
            Ok(player) => player,
            Err(error) => {
                self.notify_workspace_error("Voice", error, cx);
                return;
            }
        };
        self.workspace.voice.speech = Some(player);
        self.workspace.voice.speech_generation =
            self.workspace.voice.speech_generation.wrapping_add(1);
        let generation = self.workspace.voice.speech_generation;
        cx.spawn(async move |weak, cx| {
            let result = done.recv().await;
            let _ = weak.update(cx, |this, cx| {
                if this.workspace.voice.speech_generation != generation {
                    return;
                }
                this.workspace.voice.speech.take();
                if let Ok(Err(error)) = result {
                    this.notify_workspace_error("Voice", error, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
