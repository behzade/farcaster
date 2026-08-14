//! UI-neutral application runtime and active-session ownership.

use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use serde_json::{Value, json};

use crate::{
    conversation::ConversationState,
    protocol::{ExtensionUiResponse, Model, PromptMode, SessionState, command, prompt_command},
    rpc_process::{ProcessCommand, ProcessItem, RpcProcess},
    sessions::{SessionSummary, discover},
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
}

struct DiscoveryResult {
    generation: u64,
    result: Result<Vec<SessionSummary>, String>,
}

fn run(
    project: PathBuf,
    process_command: ProcessCommand,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: mpsc::Sender<RuntimeEvent>,
) {
    let (discovery_tx, discovery_rx) = mpsc::channel();
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
    };
    owner.load_sessions(String::new());
    owner.start_process(None);
    let mut running = true;
    while running {
        while let Ok(result) = discovery_rx.try_recv() {
            owner.apply_discovery(result);
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
        if let Some(mut old) = self.process.take() {
            let _ = old.terminate();
        }
        self.pending_prompt_id = None;
        let status = session.as_ref().map_or_else(
            || "Starting new session".into(),
            |_| "Resuming session".into(),
        );
        reset_snapshot_for_process(&mut self.snapshot, session.clone(), status);
        let _ = self.event_tx.send(RuntimeEvent::SessionReset {
            generation: self.process_generation,
        });
        self.publish();
        match RpcProcess::spawn(&self.process_command, &self.project, session.as_deref()) {
            Ok(process) => {
                self.process = Some(process);
                self.snapshot.connected = true;
                self.snapshot.status = "Loading session".into();
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
            RuntimeCommand::NewSession => self.send(command("new_session")),
            RuntimeCommand::Resume(path) => {
                if should_replace_process(self.snapshot.selected_session.as_deref(), &path) {
                    self.start_process(Some(path));
                }
            }
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
                self.snapshot.conversation.reduce(&event);
                self.snapshot.status = run_status(&self.snapshot.conversation).to_owned();
                self.publish();
            }
            ProcessItem::Stderr(chunk) => {
                self.snapshot.stderr.push_str(&chunk);
                if self.snapshot.stderr.len() > 32 * 1024 {
                    self.snapshot.stderr.drain(..16 * 1024);
                }
                self.publish();
            }
            ProcessItem::Failure(error) => self.fail(error),
        }
    }

    fn apply_response(&mut self, response: crate::protocol::RpcResponse) {
        let is_prompt_response =
            matches!(response.command.as_str(), "prompt" | "steer" | "follow_up")
                && response.id.as_ref() == self.pending_prompt_id.as_ref();
        if is_prompt_response {
            self.pending_prompt_id = None;
            self.emit_prompt_result(response.success);
        }
        if !response.success {
            self.snapshot.conversation.push_local_error(
                "Command failed",
                format!(
                    "{}: {}",
                    response.command,
                    response.error.unwrap_or_else(|| "command failed".into())
                ),
            );
            self.snapshot.status = "Command failed".into();
            self.publish();
            return;
        }
        match response.command.as_str() {
            "get_state" => match serde_json::from_value::<SessionState>(response.data) {
                Ok(state) => {
                    self.snapshot.selected_session = state
                        .session_file
                        .as_ref()
                        .map(PathBuf::from)
                        .or_else(|| self.snapshot.selected_session.clone());
                    self.snapshot.conversation.running = state.is_streaming;
                    self.snapshot.session = Some(state);
                    self.snapshot.status = "Ready".into();
                }
                Err(error) => self.fail(format!("decode get_state: {error}")),
            },
            "get_messages" => {
                let messages = response
                    .data
                    .get("messages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.snapshot.conversation.replace_history(&messages);
            }
            "get_available_models" => {
                self.snapshot.models = response
                    .data
                    .get("models")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_default();
            }
            "get_available_thinking_levels" => {
                self.snapshot.thinking_levels = response
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
            "get_session_stats" => self.snapshot.stats = response.data,
            "get_commands" => {
                self.snapshot.commands = response
                    .data
                    .get("commands")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            }
            "set_model" => {
                if let Ok(model) = serde_json::from_value::<Model>(response.data)
                    && let Some(state) = self.snapshot.session.as_mut()
                {
                    state.model = Some(model);
                }
                self.send(command("get_available_thinking_levels"));
                self.send(command("get_state"));
            }
            "set_thinking_level" => {
                self.send(command("get_state"));
            }
            "new_session" => {
                if response.data.get("cancelled").and_then(Value::as_bool) != Some(true) {
                    self.process_generation = self.process_generation.saturating_add(1);
                    self.pending_prompt_id = None;
                    reset_snapshot_for_live_session(
                        &mut self.snapshot,
                        "Loading new session".into(),
                    );
                    let _ = self.event_tx.send(RuntimeEvent::SessionReset {
                        generation: self.process_generation,
                    });
                    self.publish();
                    self.send_startup_queries();
                    self.load_sessions(String::new());
                }
            }
            "prompt" | "steer" | "follow_up" => self.snapshot.status = "Accepted".into(),
            "abort" => self.snapshot.status = "Stopping".into(),
            "compact" | "set_auto_compaction" | "set_auto_retry" | "abort_retry" => {
                self.send(command("get_state"))
            }
            _ => {}
        }
        self.publish();
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
        if let Some(mut process) = self.process.take() {
            let _ = process.terminate();
        }
        self.snapshot.connected = false;
        self.snapshot.status = "Connection failed".into();
        self.snapshot.conversation.push_transport_error(error);
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

fn should_replace_process(current: Option<&std::path::Path>, requested: &std::path::Path) -> bool {
    current != Some(requested)
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
    use std::path::Path;

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
    fn one_owner_reuses_only_the_exact_owned_session() {
        assert!(!should_replace_process(
            Some(Path::new("/a")),
            Path::new("/a")
        ));
        assert!(should_replace_process(
            Some(Path::new("/a")),
            Path::new("/b")
        ));
        assert!(should_replace_process(None, Path::new("/a")));
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
        assert_eq!(snapshot.auto_retry, false);
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
}
