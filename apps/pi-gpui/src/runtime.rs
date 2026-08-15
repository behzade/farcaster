//! UI-neutral application runtime and active-session ownership.

use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use serde_json::{Value, json};

use crate::{
    conversation::ConversationState,
    protocol::{ExtensionUiResponse, Model, PromptMode, SessionState, command, prompt_command},
    rpc_process::{ProcessCommand, ProcessItem, RpcProcess},
    sessions::{SessionSummary, discover, load_history},
};

#[derive(Clone, Debug)]
pub(crate) enum RuntimeCommand {
    Prompt { mode: PromptMode, message: String },
    Abort,
    NewSession,
    Resume(PathBuf),
    SetModel { provider: String, model_id: String },
    SetThinking(String),
    Compact,
    SetAutoCompaction(bool),
    SetAutoRetry(bool),
    AbortRetry,
    ExtensionResponse(ExtensionUiResponse),
    LoadSessions(String),
    Restart,
    Shutdown,
}

#[derive(Clone, Debug)]
pub(crate) enum RuntimeEvent {
    Snapshot {
        generation: u64,
        snapshot: Box<RuntimeSnapshot>,
    },
    SessionReset {
        generation: u64,
        preserve_submission: bool,
    },
    HistoryReset {
        generation: u64,
    },
    Sessions {
        generation: u64,
        sessions: Vec<SessionSummary>,
    },
    SessionsFailed {
        generation: u64,
        message: String,
    },
    ExtensionUi {
        generation: u64,
        request: crate::protocol::ExtensionUiRequest,
    },
    PromptResult {
        generation: u64,
        accepted: bool,
    },
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub connected: bool,
    pub status: String,
    pub session: Option<SessionState>,
    pub selected_session: Option<PathBuf>,
    pub conversation: ConversationState,
    pub models: Vec<Model>,
    pub thinking_levels: Vec<String>,
    pub stats: Value,
    pub commands: Vec<Value>,
    pub stderr: String,
    pub auto_retry: bool,
    pub history_preview: bool,
}

pub(crate) struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
}

impl RuntimeHandle {
    pub(crate) fn spawn(project: PathBuf) -> Self {
        Self::spawn_with(project, ProcessCommand::default())
    }

    pub(crate) fn spawn_with(project: PathBuf, process_command: ProcessCommand) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("pi-gpui-runtime".into())
            .spawn(move || run(project, process_command, command_rx, event_tx))
            .ok();
        Self { commands, events }
    }

    pub(crate) fn send(&self, command: RuntimeCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "Pi runtime has stopped".to_owned())
    }

    pub(crate) fn try_recv(&self) -> Result<RuntimeEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }
}

struct RuntimeOwner {
    project: PathBuf,
    process_command: ProcessCommand,
    process: Option<RpcProcess>,
    snapshot: RuntimeSnapshot,
    session_generation: u64,
    process_generation: u64,
    pending_prompt_id: Option<String>,
    event_tx: mpsc::Sender<RuntimeEvent>,
    discovery_tx: mpsc::Sender<DiscoveryResult>,
    history_tx: mpsc::Sender<HistoryResult>,
    history_generation: u64,
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<(PromptMode, String)>,
    startup_state_loaded: bool,
    startup_history_loaded: bool,
}

struct DiscoveryResult {
    generation: u64,
    result: Result<Vec<SessionSummary>, String>,
}

struct HistoryResult {
    generation: u64,
    path: PathBuf,
    result: Result<Vec<Value>, String>,
}

fn run(
    project: PathBuf,
    process_command: ProcessCommand,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: mpsc::Sender<RuntimeEvent>,
) {
    let (discovery_tx, discovery_rx) = mpsc::channel();
    let (history_tx, history_rx) = mpsc::channel();
    let mut owner = RuntimeOwner {
        project,
        process_command,
        process: None,
        snapshot: RuntimeSnapshot {
            status: "Starting Pi".into(),
            auto_retry: true,
            ..RuntimeSnapshot::default()
        },
        session_generation: 0,
        process_generation: 0,
        pending_prompt_id: None,
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 0,
        active_session: None,
        parked_snapshot: None,
        deferred_prompt: None,
        startup_state_loaded: false,
        startup_history_loaded: false,
    };
    owner.load_sessions(String::new());
    owner.start_process(None);
    let mut running = true;
    while running {
        while let Ok(result) = discovery_rx.try_recv() {
            owner.apply_discovery(result);
        }
        while let Ok(result) = history_rx.try_recv() {
            owner.apply_history(result);
        }
        while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
            owner.apply_process_item(item);
        }
        match command_rx.recv_timeout(Duration::from_millis(12)) {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => owner.apply_command(command),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => running = false,
        }
    }
    if let Some(mut process) = owner.process.take() {
        let _ = process.terminate();
    }
    let _ = owner.event_tx.send(RuntimeEvent::Stopped);
}

impl RuntimeOwner {
    fn start_process(&mut self, session: Option<PathBuf>) {
        self.process_generation = self.process_generation.saturating_add(1);
        let preserve_submission = self.deferred_prompt.is_some();
        let keep_preview = preserve_submission && self.snapshot.history_preview;
        if let Some(mut old) = self.process.take() {
            let _ = old.terminate();
        }
        self.active_session = session.clone();
        self.parked_snapshot = None;
        self.startup_state_loaded = false;
        self.startup_history_loaded = false;
        self.pending_prompt_id = None;
        let status = session.as_ref().map_or_else(
            || "Starting new session".into(),
            |_| "Resuming session".into(),
        );
        if keep_preview {
            let mut loading = RuntimeSnapshot {
                auto_retry: self.snapshot.auto_retry,
                ..RuntimeSnapshot::default()
            };
            reset_snapshot_for_process(&mut loading, session.clone(), status);
            self.parked_snapshot = Some(loading);
        } else {
            reset_snapshot_for_process(&mut self.snapshot, session.clone(), status);
        }
        let _ = self.event_tx.send(RuntimeEvent::SessionReset {
            generation: self.process_generation,
            preserve_submission,
        });
        self.publish();
        match RpcProcess::spawn(&self.process_command, &self.project, session.as_deref()) {
            Ok(process) => {
                self.process = Some(process);
                let snapshot = self.active_snapshot_mut();
                snapshot.connected = true;
                snapshot.status = "Loading session".into();
                self.send_startup_queries();
            }
            Err(error) => self.fail(error),
        }
        self.publish();
    }

    fn send_startup_queries(&mut self) {
        for kind in [
            "get_state",
            "get_messages",
            "get_session_stats",
            "get_available_models",
            "get_available_thinking_levels",
            "get_commands",
        ] {
            self.send(command(kind));
        }
    }

    fn apply_command(&mut self, runtime_command: RuntimeCommand) {
        match runtime_command {
            RuntimeCommand::Prompt { mode, message } => self.send_prompt(mode, message),
            RuntimeCommand::Abort => self.send(command("abort")),
            RuntimeCommand::NewSession => {
                if self.snapshot.history_preview {
                    self.start_process(None);
                } else {
                    self.send(command("new_session"));
                }
            }
            RuntimeCommand::Resume(path) => self.preview_history(path),
            RuntimeCommand::SetModel { provider, model_id } => {
                self.send(json!({"type":"set_model","provider":provider,"modelId":model_id}))
            }
            RuntimeCommand::SetThinking(level) => {
                self.send(json!({"type":"set_thinking_level","level":level}))
            }
            RuntimeCommand::Compact => self.send(command("compact")),
            RuntimeCommand::SetAutoCompaction(enabled) => {
                self.send(json!({"type":"set_auto_compaction","enabled":enabled}))
            }
            RuntimeCommand::SetAutoRetry(enabled) => {
                self.snapshot.auto_retry = enabled;
                self.send(json!({"type":"set_auto_retry","enabled":enabled}));
            }
            RuntimeCommand::AbortRetry => self.send(command("abort_retry")),
            RuntimeCommand::ExtensionResponse(response) => {
                if let Some(process) = self.process.as_mut()
                    && let Err(error) = process.send_extension_response(response)
                {
                    self.fail(error);
                }
            }
            RuntimeCommand::LoadSessions(query) => self.load_sessions(query),
            RuntimeCommand::Restart => self.start_process(self.snapshot.selected_session.clone()),
            RuntimeCommand::Shutdown => {}
        }
    }

    fn send(&mut self, command: Value) {
        let command_name = command
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("command")
            .to_owned();
        match self
            .process
            .as_mut()
            .map(|process| process.send_command(command))
        {
            Some(Ok(_)) => {}
            Some(Err(error)) => self.fail(error),
            None => self.fail(format!("Cannot send {command_name}: Pi is not connected")),
        }
    }

    fn send_prompt(&mut self, mode: PromptMode, message: String) {
        if self.snapshot.history_preview {
            let Some(path) = self.snapshot.selected_session.clone() else {
                self.reject_prompt("No session is selected".into());
                return;
            };
            self.deferred_prompt = Some((mode, message));
            self.start_process(Some(path));
            return;
        }
        if self.pending_prompt_id.is_some() {
            self.reject_prompt("Another composer submission is awaiting Pi acceptance".into());
            return;
        }
        if !can_send_prompt(mode, self.snapshot.conversation.running) {
            self.reject_prompt(
                "A normal prompt cannot be sent while Pi is working; use Steer or Follow-up".into(),
            );
            return;
        }
        let command = prompt_command(mode, message);
        match self
            .process
            .as_mut()
            .map(|process| process.send_command(command))
        {
            Some(Ok(id)) => self.pending_prompt_id = Some(id),
            Some(Err(error)) => {
                self.emit_prompt_result(false);
                self.fail(error);
            }
            None => {
                self.emit_prompt_result(false);
                self.fail("Cannot send prompt: Pi is not connected".into());
            }
        }
    }

    fn reject_prompt(&mut self, message: String) {
        self.snapshot
            .conversation
            .push_local_error("Prompt not sent", message);
        self.snapshot.status = "Prompt not sent".into();
        self.emit_prompt_result(false);
        self.publish();
    }

    fn emit_prompt_result(&self, accepted: bool) {
        let _ = self.event_tx.send(RuntimeEvent::PromptResult {
            generation: self.process_generation,
            accepted,
        });
    }

    fn apply_process_item(&mut self, item: ProcessItem) {
        match item {
            ProcessItem::Response(response) => self.apply_response(response),
            ProcessItem::ExtensionUi(request) => {
                let _ = self.event_tx.send(RuntimeEvent::ExtensionUi {
                    generation: self.process_generation,
                    request,
                });
            }
            ProcessItem::Event(event) => {
                let settled = event.get("type").and_then(Value::as_str) == Some("agent_settled");
                let previewing = self.parked_snapshot.is_some();
                let snapshot = self.active_snapshot_mut();
                snapshot.conversation.reduce(&event);
                snapshot.status = run_status(&snapshot.conversation).to_owned();
                if !previewing {
                    self.publish();
                }
                if settled {
                    self.send(command("get_session_stats"));
                    self.load_sessions(String::new());
                }
            }
            ProcessItem::Stderr(chunk) => {
                let previewing = self.parked_snapshot.is_some();
                let snapshot = self.active_snapshot_mut();
                snapshot.stderr.push_str(&chunk);
                if snapshot.stderr.len() > 32 * 1024 {
                    snapshot.stderr.drain(..16 * 1024);
                }
                if !previewing {
                    self.publish();
                }
            }
            ProcessItem::Failure(error) => self.fail(error),
        }
    }

    fn active_snapshot_mut(&mut self) -> &mut RuntimeSnapshot {
        self.parked_snapshot.as_mut().unwrap_or(&mut self.snapshot)
    }

    fn preview_history(&mut self, path: PathBuf) {
        self.history_generation = self.history_generation.saturating_add(1);
        if self.snapshot.selected_session.as_deref() == Some(path.as_path()) {
            return;
        }
        let active = self.parked_snapshot.as_ref().unwrap_or(&self.snapshot);
        if active.conversation.running
            || active.conversation.retrying
            || active.conversation.compacting
            || !active.conversation.queue.steering.is_empty()
            || !active.conversation.queue.follow_up.is_empty()
            || self.pending_prompt_id.is_some()
        {
            self.snapshot.status = "Finish the active run before switching history".into();
            self.publish();
            return;
        }
        if self.active_session.as_deref() == Some(path.as_path()) {
            if let Some(snapshot) = self.parked_snapshot.take() {
                self.snapshot = snapshot;
                let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
                    generation: self.process_generation,
                });
                self.publish();
            }
            return;
        }
        let generation = self.history_generation;
        let sender = self.history_tx.clone();
        thread::Builder::new()
            .name("pi-gpui-history".into())
            .spawn(move || {
                let result = load_history(&path);
                let _ = sender.send(HistoryResult {
                    generation,
                    path,
                    result,
                });
            })
            .ok();
    }

    fn apply_history(&mut self, result: HistoryResult) {
        if result.generation != self.history_generation {
            return;
        }
        let messages = match result.result {
            Ok(messages) => messages,
            Err(error) => {
                self.snapshot.status = "Could not load history".into();
                self.snapshot
                    .conversation
                    .push_local_error("History unavailable", error);
                self.publish();
                return;
            }
        };
        if self.parked_snapshot.is_none() {
            self.parked_snapshot = Some(std::mem::take(&mut self.snapshot));
        }
        let auto_retry = self
            .parked_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.auto_retry);
        let mut conversation = ConversationState::default();
        conversation.replace_history(&messages);
        self.snapshot = RuntimeSnapshot {
            connected: true,
            status: "History preview · Pi loads when you send".into(),
            selected_session: Some(result.path),
            conversation,
            auto_retry,
            history_preview: true,
            ..RuntimeSnapshot::default()
        };
        let _ = self.event_tx.send(RuntimeEvent::HistoryReset {
            generation: self.process_generation,
        });
        self.publish();
    }

    fn apply_response(&mut self, response: crate::protocol::RpcResponse) {
        let response_command = response.command.clone();
        let is_prompt_response =
            matches!(response.command.as_str(), "prompt" | "steer" | "follow_up")
                && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            self.pending_prompt_id = None;
            self.emit_prompt_result(response.success);
        }
        if !response.success {
            let blocks_resume = self.deferred_prompt.is_some()
                && matches!(response.command.as_str(), "get_state" | "get_messages");
            let snapshot = self.active_snapshot_mut();
            snapshot.conversation.push_local_error(
                "Command failed",
                format!(
                    "{}: {}",
                    response.command,
                    response.error.unwrap_or_else(|| "command failed".into())
                ),
            );
            snapshot.status = "Command failed".into();
            if blocks_resume {
                self.deferred_prompt = None;
                self.emit_prompt_result(false);
                if let Some(snapshot) = self.parked_snapshot.take() {
                    self.snapshot = snapshot;
                }
            }
            if self.parked_snapshot.is_none() {
                self.publish();
            }
            return;
        }
        match response.command.as_str() {
            "get_state" => match serde_json::from_value::<SessionState>(response.data) {
                Ok(state) => {
                    let selected_session = state
                        .session_file
                        .as_ref()
                        .map(PathBuf::from)
                        .or_else(|| self.active_session.clone());
                    self.active_session = selected_session.clone();
                    let snapshot = self.active_snapshot_mut();
                    snapshot.selected_session = selected_session;
                    snapshot.conversation.running = state.is_streaming;
                    snapshot.session = Some(state);
                    snapshot.status = "Ready".into();
                    self.startup_state_loaded = true;
                }
                Err(error) => {
                    self.fail(format!("decode get_state: {error}"));
                    return;
                }
            },
            "get_messages" => {
                let messages = response
                    .data
                    .get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.active_snapshot_mut()
                    .conversation
                    .replace_history(&messages);
                self.startup_history_loaded = true;
            }
            "get_available_models" => {
                self.active_snapshot_mut().models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
            }
            "get_available_thinking_levels" => {
                self.active_snapshot_mut().thinking_levels = response
                    .data
                    .get("levels")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "get_session_stats" => self.active_snapshot_mut().stats = response.data,
            "get_commands" => {
                self.active_snapshot_mut().commands = response
                    .data
                    .get("commands")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            }
            "set_model" => {
                if let Ok(model) = serde_json::from_value::<Model>(response.data)
                    && let Some(state) = self.active_snapshot_mut().session.as_mut()
                {
                    state.model = Some(model);
                }
                self.send(command("get_available_thinking_levels"));
                self.send(command("get_state"));
            }
            "set_thinking_level" => {
                self.send(command("get_state"));
            }
            "new_session"
                if response.data.get("cancelled").and_then(Value::as_bool) != Some(true) =>
            {
                self.process_generation = self.process_generation.saturating_add(1);
                self.active_session = None;
                self.pending_prompt_id = None;
                reset_snapshot_for_live_session(
                    self.active_snapshot_mut(),
                    "Loading new session".into(),
                );
                let _ = self.event_tx.send(RuntimeEvent::SessionReset {
                    generation: self.process_generation,
                    preserve_submission: false,
                });
                self.publish();
                self.send_startup_queries();
                self.load_sessions(String::new());
            }
            "prompt" | "steer" | "follow_up" => {
                self.active_snapshot_mut().status = "Accepted".into()
            }
            "abort" => self.active_snapshot_mut().status = "Stopping".into(),
            "compact" | "set_auto_compaction" | "set_auto_retry" | "abort_retry" => {
                self.send(command("get_state"))
            }
            _ => {}
        }
        if matches!(response_command.as_str(), "get_state" | "get_messages") {
            self.maybe_send_deferred_prompt();
        }
        if self.parked_snapshot.is_none() {
            self.publish();
        }
    }

    fn maybe_send_deferred_prompt(&mut self) {
        if !self.startup_state_loaded || !self.startup_history_loaded {
            return;
        }
        if let Some((mode, message)) = self.deferred_prompt.take() {
            if self.snapshot.history_preview
                && let Some(snapshot) = self.parked_snapshot.take()
            {
                self.snapshot = snapshot;
            }
            self.send_prompt(mode, message);
        }
    }

    fn load_sessions(&mut self, query: String) {
        self.session_generation = self.session_generation.saturating_add(1);
        let generation = self.session_generation;
        let project = self.project.clone();
        let sender = self.discovery_tx.clone();
        thread::Builder::new()
            .name("pi-gpui-sessions".into())
            .spawn(move || {
                let _ = sender.send(DiscoveryResult {
                    generation,
                    result: discover(&project, &query),
                });
            })
            .ok();
    }

    fn apply_discovery(&self, result: DiscoveryResult) {
        if result.generation != self.session_generation {
            return;
        }
        let event = match result.result {
            Ok(sessions) => RuntimeEvent::Sessions {
                generation: result.generation,
                sessions,
            },
            Err(message) => RuntimeEvent::SessionsFailed {
                generation: result.generation,
                message,
            },
        };
        let _ = self.event_tx.send(event);
    }

    fn fail(&mut self, error: String) {
        if self.pending_prompt_id.take().is_some() {
            self.emit_prompt_result(false);
        }
        if self.deferred_prompt.take().is_some() {
            self.emit_prompt_result(false);
        }
        if let Some(mut process) = self.process.take() {
            let _ = process.terminate();
        }
        let previewing = self.parked_snapshot.is_some();
        let snapshot = self.active_snapshot_mut();
        snapshot.connected = false;
        snapshot.status = "Connection failed".into();
        snapshot.conversation.push_transport_error(error);
        if previewing && let Some(snapshot) = self.parked_snapshot.take() {
            self.snapshot = snapshot;
        }
        self.publish();
    }

    fn publish(&self) {
        let _ = self.event_tx.send(RuntimeEvent::Snapshot {
            generation: self.process_generation,
            snapshot: Box::new(self.snapshot.clone()),
        });
    }
}

fn reset_snapshot_for_process(
    snapshot: &mut RuntimeSnapshot,
    selected_session: Option<PathBuf>,
    status: String,
) {
    let auto_retry = snapshot.auto_retry;
    *snapshot = RuntimeSnapshot {
        status,
        selected_session,
        auto_retry,
        ..RuntimeSnapshot::default()
    };
}

fn reset_snapshot_for_live_session(snapshot: &mut RuntimeSnapshot, status: String) {
    let auto_retry = snapshot.auto_retry;
    *snapshot = RuntimeSnapshot {
        connected: true,
        status,
        auto_retry,
        ..RuntimeSnapshot::default()
    };
}

const fn can_send_prompt(mode: PromptMode, running: bool) -> bool {
    !running || !matches!(mode, PromptMode::Normal)
}

fn run_status(conversation: &ConversationState) -> &'static str {
    if conversation.compacting {
        "Compacting"
    } else if conversation.retrying {
        "Retrying"
    } else if conversation.running {
        "Working"
    } else if conversation.settled {
        "Ready"
    } else {
        "Idle"
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt as _,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn agent_end_does_not_report_ready_until_settled() {
        let mut conversation = ConversationState::default();
        conversation.reduce(&json!({"type":"agent_start"}));
        conversation.reduce(&json!({"type":"agent_end","willRetry":false}));
        assert_eq!(run_status(&conversation), "Working");
        conversation.reduce(&json!({"type":"agent_settled"}));
        assert_eq!(run_status(&conversation), "Ready");
    }

    #[test]
    fn streaming_accepts_only_steer_and_follow_up_composer_modes() {
        assert!(!can_send_prompt(PromptMode::Normal, true));
        assert!(can_send_prompt(PromptMode::Steer, true));
        assert!(can_send_prompt(PromptMode::FollowUp, true));
        assert!(can_send_prompt(PromptMode::Normal, false));
    }

    #[test]
    fn process_replacement_clears_all_session_owned_snapshot_state() {
        let mut snapshot = RuntimeSnapshot {
            connected: true,
            status: "old".into(),
            session: serde_json::from_value(json!({
                "model": null,
                "thinkingLevel": "high",
                "isStreaming": true,
                "isCompacting": true,
                "sessionFile": "/old",
                "sessionId": "old",
                "autoCompactionEnabled": true,
                "messageCount": 9,
                "pendingMessageCount": 2
            }))
            .ok(),
            selected_session: Some(PathBuf::from("/old")),
            models: vec![Model {
                id: "old".into(),
                name: "Old".into(),
                provider: "test".into(),
                reasoning: false,
            }],
            thinking_levels: vec!["high".into()],
            stats: json!({"tokens": 10}),
            commands: vec![json!({"name": "old"})],
            stderr: "old stderr".into(),
            auto_retry: false,
            ..RuntimeSnapshot::default()
        };
        snapshot
            .conversation
            .reduce(&json!({"type": "agent_start"}));
        snapshot.conversation.reduce(&json!({
            "type": "queue_update",
            "steering": ["old"],
            "followUp": ["later"]
        }));

        reset_snapshot_for_process(
            &mut snapshot,
            Some(PathBuf::from("/new")),
            "Resuming session".into(),
        );

        assert!(!snapshot.connected);
        assert_eq!(snapshot.status, "Resuming session");
        assert_eq!(snapshot.selected_session, Some(PathBuf::from("/new")));
        assert!(!snapshot.auto_retry);
        assert_eq!(snapshot.session, None);
        assert!(snapshot.models.is_empty());
        assert!(snapshot.thinking_levels.is_empty());
        assert_eq!(snapshot.stats, Value::Null);
        assert!(snapshot.commands.is_empty());
        assert!(snapshot.stderr.is_empty());
        assert_eq!(snapshot.conversation, ConversationState::default());
    }

    #[test]
    fn failed_resume_publishes_no_state_from_the_previous_process() {
        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        let mut owner = RuntimeOwner {
            project: std::env::temp_dir(),
            process_command: ProcessCommand {
                program: PathBuf::from("/definitely/missing/pi-gpui-test-command"),
                prefix_args: Vec::new(),
            },
            process: None,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "old".into(),
                selected_session: Some(PathBuf::from("/old")),
                models: vec![Model {
                    id: "old".into(),
                    name: "Old".into(),
                    provider: "test".into(),
                    reasoning: false,
                }],
                thinking_levels: vec!["high".into()],
                stats: json!({"old": true}),
                commands: vec![json!({"name": "old"})],
                stderr: "old stderr".into(),
                auto_retry: true,
                ..RuntimeSnapshot::default()
            },
            session_generation: 0,
            process_generation: 4,
            pending_prompt_id: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            active_session: Some(PathBuf::from("/old")),
            parked_snapshot: None,
            deferred_prompt: None,
            startup_state_loaded: false,
            startup_history_loaded: false,
        };
        owner.snapshot.conversation.reduce(&json!({
            "type": "queue_update",
            "steering": ["old"],
            "followUp": []
        }));

        owner.start_process(Some(PathBuf::from("/new")));

        let snapshots = event_rx
            .try_iter()
            .filter_map(|event| match event {
                RuntimeEvent::Snapshot { snapshot, .. } => Some(*snapshot),
                _ => None,
            })
            .collect::<Vec<_>>();
        let latest = snapshots.last().expect("failed replacement should publish");
        assert_eq!(latest.selected_session, Some(PathBuf::from("/new")));
        assert!(latest.models.is_empty());
        assert!(latest.thinking_levels.is_empty());
        assert_eq!(latest.stats, Value::Null);
        assert!(latest.commands.is_empty());
        assert!(latest.stderr.is_empty());
        assert!(latest.conversation.queue.steering.is_empty());
        assert!(
            latest
                .conversation
                .items
                .iter()
                .any(|item| item.text.contains("definitely/missing"))
        );
    }

    #[test]
    fn history_preview_keeps_pi_until_a_prompt_resumes_the_session() -> Result<(), String> {
        let temp = tempdir().map_err(|error| error.to_string())?;
        let script = temp.path().join("fake-pi.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))
            .map_err(|error| error.to_string())?;
        let mut permissions = fs::metadata(&script)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).map_err(|error| error.to_string())?;
        let process_command = ProcessCommand {
            program: script,
            prefix_args: vec!["quiet".into()],
        };
        let process = RpcProcess::spawn(&process_command, temp.path(), None)?;
        let (event_tx, event_rx) = mpsc::channel();
        let (discovery_tx, _discovery_rx) = mpsc::channel();
        let (history_tx, _history_rx) = mpsc::channel();
        let old_path = PathBuf::from("/old");
        let new_path = PathBuf::from("/new");
        let mut owner = RuntimeOwner {
            project: temp.path().to_path_buf(),
            process_command,
            process: Some(process),
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "Ready".into(),
                selected_session: Some(old_path.clone()),
                ..RuntimeSnapshot::default()
            },
            session_generation: 0,
            process_generation: 3,
            pending_prompt_id: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 1,
            active_session: Some(old_path.clone()),
            parked_snapshot: None,
            deferred_prompt: None,
            startup_state_loaded: false,
            startup_history_loaded: false,
        };

        owner.preview_history(old_path.clone());
        owner.apply_history(HistoryResult {
            generation: 1,
            path: new_path.clone(),
            result: Ok(vec![json!({"role":"user","content":"previewed"})]),
        });
        assert!(!owner.snapshot.history_preview);
        owner.apply_history(HistoryResult {
            generation: 2,
            path: new_path.clone(),
            result: Ok(vec![json!({"role":"user","content":"previewed"})]),
        });

        assert!(owner.process.is_some());
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.selected_session, Some(new_path.clone()));
        assert_eq!(
            owner
                .parked_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.selected_session.clone()),
            Some(old_path)
        );

        let _ = event_rx.try_iter().count();
        owner.send_prompt(PromptMode::Normal, "continue".into());
        assert!(owner.snapshot.history_preview);
        assert_eq!(owner.snapshot.conversation.items[0].text, "previewed");
        assert_eq!(owner.active_session, Some(new_path));
        assert!(owner.deferred_prompt.is_some());
        let resume_events = event_rx.try_iter().collect::<Vec<_>>();
        assert!(resume_events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SessionReset {
                preserve_submission: true,
                ..
            }
        )));
        assert!(
            resume_events
                .iter()
                .filter_map(|event| match event {
                    RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                    _ => None,
                })
                .all(|snapshot| {
                    snapshot.history_preview
                        && snapshot
                            .conversation
                            .items
                            .first()
                            .map(|item| item.text.as_str())
                            == Some("previewed")
                })
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            while let Some(item) = owner.process.as_mut().and_then(RpcProcess::try_next) {
                owner.apply_process_item(item);
            }
            if owner.deferred_prompt.is_none() && owner.pending_prompt_id.is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(owner.deferred_prompt.is_none());
        assert!(owner.pending_prompt_id.is_none());
        assert!(!owner.snapshot.history_preview);
        assert!(
            event_rx
                .try_iter()
                .any(|event| matches!(event, RuntimeEvent::PromptResult { accepted: true, .. }))
        );
        Ok(())
    }
}
