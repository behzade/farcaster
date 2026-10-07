use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::client::Client;

pub(super) struct Bridge {
    child: Child,
    pub(super) client: Client,
    api_key: Option<String>,
    temporary_store: Option<tempfile::TempDir>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ready {
    schema_version: u32,
    transport: String,
    protocol: String,
    url: String,
    auth_token_file: std::path::PathBuf,
}

impl Bridge {
    pub(super) fn start(config: &crate::AgentLaunchConfig, project: &Path) -> Result<Self, String> {
        Self::launch(config, project, false, false)
    }

    pub(super) fn start_live(
        config: &crate::AgentLaunchConfig,
        project: &Path,
        ephemeral: bool,
    ) -> Result<Self, String> {
        Self::launch(config, project, true, ephemeral)
    }

    fn launch(
        config: &crate::AgentLaunchConfig,
        project: &Path,
        live: bool,
        ephemeral: bool,
    ) -> Result<Self, String> {
        let mut command = config.command(project)?;
        let api_key = super::auth::api_key(config, &command)?;
        if let Some(key) = &api_key {
            command.env("CURSOR_API_KEY", key);
        }
        command
            .args(["--workspace"])
            .arg(project)
            .env("CURSOR_SDK_CLIENT_LANGUAGE", "rust");
        if let Some(root) = config.locator_root() {
            command
                .arg("--state-root")
                .arg(root.join("cursor-sdk-store"));
        }
        let temporary_store = ephemeral
            .then(|| {
                tempfile::Builder::new()
                    .prefix("farcaster-cursor-title-")
                    .tempdir()
            })
            .transpose()
            .map_err(|error| format!("Create Cursor temporary store: {error}"))?;
        if let Some(store) = &temporary_store {
            command.arg("--temporary-store").arg(store.path());
        }
        if live {
            let mut helper =
                super::auth::sdk_command(&command, project, include_str!("runtime.mjs"))?;
            helper.args(command.get_args());
            command = helper;
        }
        let mut bridge = Self::from_command(command, api_key)?;
        bridge.temporary_store = temporary_store;
        Ok(bridge)
    }

    fn from_command(
        mut command: std::process::Command,
        api_key: Option<String>,
    ) -> Result<Self, String> {
        let mut timing = super::timing::Call::start("Bridge.start");
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Start Cursor SDK bridge: {e}"))?;
        let (sender, receiver) = mpsc::channel();
        let stderr = child.stderr.take().expect("piped bridge stderr");
        let reader = thread::Builder::new()
            .name("cursor-sdk-stderr".into())
            .spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut announced = false;
                loop {
                    let mut line = String::new();
                    match reader.by_ref().take(65537).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    if !announced && let Some(json) = line.strip_prefix("cursor-sdk-bridge ready ")
                    {
                        let ready = serde_json::from_str::<Ready>(json)
                            .map_err(|e| format!("Decode Cursor SDK handshake: {e}"));
                        let _ = sender.send(ready);
                        announced = true;
                    }
                    if let Some(json) = line.strip_prefix("cursor-sdk-timing ")
                        && let Ok(event) = serde_json::from_str::<Value>(json)
                    {
                        super::timing::log_event(&event);
                    }
                    // Other SDK stderr may contain credentials; keep draining without logging it.
                }
            });
        if let Err(error) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Start Cursor SDK stderr reader: {error}"));
        }
        let setup = (|| {
            let ready = receiver
                .recv_timeout(Duration::from_secs(30))
                .map_err(|e| format!("Cursor SDK bridge did not become ready: {e}"))??;
            if ready.schema_version != 1 || ready.transport != "tcp" || ready.protocol != "connect"
            {
                return Err("Unsupported Cursor SDK bridge handshake".into());
            }
            let token = std::fs::read_to_string(ready.auth_token_file)
                .map_err(|e| format!("Read Cursor SDK bridge token: {e}"))?;
            let client = Client::new(&ready.url, token.trim().to_owned())?;
            let version = client.call("SdkBridgeControlService", "GetVersion", json!({}))?;
            if version["protocolVersion"] != "sdk.v1" {
                return Err("Cursor SDK bridge must support sdk.v1".into());
            }
            Ok(client)
        })();
        timing.finish(setup.is_ok());
        match setup {
            Ok(client) => Ok(Self {
                child,
                client,
                api_key,
                temporary_store: None,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub(super) fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub(super) fn is_ephemeral(&self) -> bool {
        self.temporary_store.is_some()
    }

    pub(super) fn agent(&self, method: &str, body: Value) -> Result<Value, String> {
        self.client.call("SdkAgentService", method, body)
    }

    pub(super) fn models(&self) -> Result<Vec<Value>, String> {
        let mut options = json!({});
        if let Some(key) = self.api_key() {
            options["apiKey"] = key.into();
        }
        let response =
            self.client
                .call("SdkCursorService", "ListModels", json!({"options":options}))?;
        response["items"]
            .as_array()
            .cloned()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| "Cursor SDK returned no models".into())
    }

    pub(super) fn has_exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub(super) fn close(&mut self) -> Result<(), String> {
        self.stop()?;
        if let Some(store) = self.temporary_store.take() {
            store
                .close()
                .map_err(|error| format!("Remove Cursor temporary store: {error}"))?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.has_exited() {
            return Ok(());
        }
        let _ = self.client.call(
            "SdkBridgeControlService",
            "Shutdown",
            json!({"graceSeconds":0}),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.has_exited() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.child.kill().map_err(|e| e.to_string())?;
        self.child.wait().map(|_| ()).map_err(|e| e.to_string())
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
impl Bridge {
    pub(super) fn fixture(client: Client, api_key: Option<String>) -> Self {
        Self {
            child: std::process::Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("fixture process"),
            client,
            api_key,
            temporary_store: None,
        }
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
