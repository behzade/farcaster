use std::{
    collections::VecDeque,
    process::Stdio,
    sync::mpsc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use super::server::OpenCodeServerProcess;
use crate::{
    access::PreparedCommand,
    agents::AgentProcessCommand,
    modules::agents::adapter::farcaster_mcp,
    workers::{
        WorkerContext, WorkerEvent, WorkerInputResponse, WorkerLaunch, WorkerSendMode,
        WorkerSession, WorkerSessionFactory,
    },
};

#[derive(Clone)]
pub(crate) struct OpenCodeWorkerFactory {
    command: AgentProcessCommand,
}

impl OpenCodeWorkerFactory {
    pub(crate) fn new(command: AgentProcessCommand) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for OpenCodeWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("OpenCode worker provider and model must be supplied together".into());
        }
        let mut sandbox = self.command.command(&launch.project)?;
        let caller_identity = crate::workers::CallerRegistry::shared().issue();
        configure_farcaster_mcp(&mut sandbox.command, caller_identity.token())?;
        let password = worker_password()?;
        let child = sandbox
            .command
            .args(["serve", "--stdio"])
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("start OpenCode worker server: {error}"))?;
        let server = OpenCodeServerProcess::attach(child, "opencode", password)?;
        let mut client = server.client();
        let selected_model = launch
            .provider
            .as_deref()
            .zip(launch.model.as_deref())
            .map(|(provider, model)| (provider, model, launch.effort.as_deref()));
        let session = match launch.context {
            WorkerContext::Fresh => client.create_session(
                &launch.project.to_string_lossy(),
                Some(&launch.parent_session),
                selected_model,
            )?,
            WorkerContext::Session { session_locator } => {
                if session_locator != launch.parent_session {
                    return Err(
                        "OpenCode workers cannot inherit context from a session other than their parent"
                            .into(),
                    );
                }
                client.fork_session(&session_locator, selected_model)?
            }
        };
        let session_id = session.id;
        caller_identity.bind(session_id.clone());
        Ok(Box::new(OpenCodeWorkerSession {
            _caller_identity: caller_identity,
            _sandbox: sandbox,
            server,
            session_id: session_id.clone(),
            generation: 0,
            completions: None,
            pending: VecDeque::from([WorkerEvent::SessionChanged {
                locator: session_id,
            }]),
        }))
    }
}

struct OpenCodeWorkerSession {
    _caller_identity: crate::workers::CallerIdentity,
    _sandbox: PreparedCommand,
    server: OpenCodeServerProcess,
    session_id: String,
    generation: u64,
    completions: Option<mpsc::Receiver<(u64, Result<String, String>)>>,
    pending: VecDeque<WorkerEvent>,
}

impl WorkerSession for OpenCodeWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        let delivery = match mode {
            WorkerSendMode::Prompt | WorkerSendMode::Queue => {
                super::contract::OpenCodeDelivery::Queue
            }
            WorkerSendMode::Steer => super::contract::OpenCodeDelivery::Steer,
        };
        self.server
            .client()
            .prompt(&self.session_id, &message, Vec::new(), delivery)?;
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let session_id = self.session_id.clone();
        let mut client = self.server.client();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name(format!("opencode-worker-{session_id}"))
            .spawn(move || {
                let result = client
                    .wait_session(&session_id)
                    .and_then(|()| client.context(&session_id))
                    .map(|context| final_assistant_text(&context));
                let _ = sender.send((generation, result));
            })
            .map_err(|error| format!("watch OpenCode worker: {error}"))?;
        self.completions = Some(receiver);
        self.pending.push_back(WorkerEvent::Started);
        Ok(())
    }

    fn respond(&mut self, _response: WorkerInputResponse) -> Result<(), String> {
        Err("OpenCode worker interaction responses are not supported yet".into())
    }

    fn abort(&mut self) -> Result<(), String> {
        self.generation = self.generation.saturating_add(1);
        self.server.client().interrupt(&self.session_id)
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        let completion = self.completions.as_ref()?.try_recv().ok()?;
        if completion.0 != self.generation {
            return None;
        }
        self.completions = None;
        Some(match completion.1 {
            Ok(output) => WorkerEvent::Settled { output },
            Err(error) => WorkerEvent::Failed(error),
        })
    }

    fn close(&mut self) -> Result<(), String> {
        self.server.terminate()
    }
}

fn configure_farcaster_mcp(
    command: &mut std::process::Command,
    caller_token: &str,
) -> Result<(), String> {
    let existing = command
        .get_envs()
        .find(|(name, _)| *name == "OPENCODE_CONFIG_CONTENT")
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned());
    let mut config = existing.map_or_else(
        || Ok(serde_json::json!({})),
        |value| {
            serde_json::from_str::<Value>(&value)
                .map_err(|error| format!("parse OPENCODE_CONFIG_CONTENT: {error}"))
        },
    )?;
    if !config.is_object() {
        return Err("OPENCODE_CONFIG_CONTENT must be a JSON object".into());
    }
    merge_json(
        &mut config,
        serde_json::json!({
            "mcp": {
                "servers": {
                    "farcaster": {
                        "type": "remote",
                        "url": farcaster_mcp::URL,
                        "headers": {(farcaster_mcp::CALLER_HEADER): caller_token},
                        "oauth": false,
                        "codemode": false
                    }
                }
            }
        }),
    );
    command.env(
        "OPENCODE_CONFIG_CONTENT",
        serde_json::to_string(&config)
            .map_err(|error| format!("encode OpenCode MCP configuration: {error}"))?,
    );
    Ok(())
}

fn merge_json(target: &mut Value, overlay: Value) {
    match (target, overlay) {
        (Value::Object(target), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge_json(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, overlay) => *target = overlay,
    }
}

fn final_assistant_text(context: &[Value]) -> String {
    context
        .iter()
        .rev()
        .find(|message| message["type"].as_str() == Some("assistant"))
        .and_then(|message| message["content"].as_array())
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    (part["type"].as_str() == Some("text"))
                        .then(|| part["text"].as_str())
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn worker_password() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    Ok(format!("farcaster-{}-{nanos}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_startup_merges_direct_farcaster_mcp() {
        let mut command = std::process::Command::new("opencode2");
        command.env(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"model":"provider/model","mcp":{"servers":{"other":{"type":"remote","url":"https://example.test/mcp"}}}}"#,
        );
        configure_farcaster_mcp(&mut command, "caller-1").expect("MCP config");
        let value = command
            .get_envs()
            .find(|(name, _)| *name == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .and_then(|value| serde_json::from_str::<Value>(&value.to_string_lossy()).ok())
            .expect("inline config");
        assert_eq!(value["model"], "provider/model");
        assert_eq!(
            value["mcp"]["servers"]["other"]["url"],
            "https://example.test/mcp"
        );
        assert_eq!(
            value["mcp"]["servers"]["farcaster"]["url"],
            farcaster_mcp::URL
        );
        assert_eq!(
            value["mcp"]["servers"]["farcaster"]["headers"][farcaster_mcp::CALLER_HEADER],
            "caller-1"
        );
        assert_eq!(value["mcp"]["servers"]["farcaster"]["codemode"], false);
        assert_eq!(value["mcp"]["servers"]["farcaster"]["oauth"], false);
    }

    #[test]
    fn extracts_the_last_assistant_text() {
        let context = [
            json!({"type":"assistant","content":[{"type":"text","text":"old"}]}),
            json!({"type":"user","text":"next"}),
            json!({"type":"assistant","content":[{"type":"reasoning","text":"hidden"},{"type":"text","text":"done"}]}),
        ];
        assert_eq!(final_assistant_text(&context), "done");
    }
}
