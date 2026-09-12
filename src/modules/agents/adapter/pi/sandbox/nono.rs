use std::{io::Read as _, path::Path, time::Duration};

use serde::Deserialize;
use serde_json::json;

use super::{HarnessAccessMode, PiCommand, PiRpcProcess, PiSandboxAdapter};
use crate::agents::{
    SessionEvent,
    extensions::{ExtensionUiRequest, SlashCommandSource},
};
const STATUS_KEY: &str = "\u{1f}pi-gpui-sandbox-mode\u{1f}";

pub(super) struct Nono;

impl PiSandboxAdapter for Nono {
    fn id(&self) -> &'static str {
        "pi-nono"
    }

    fn control_command<'a>(&self, commands: &'a [PiCommand]) -> Result<Option<&'a str>, String> {
        control_command(commands)
    }

    fn access_modes(&self) -> &'static [HarnessAccessMode] {
        &[HarnessAccessMode::Sandboxed, HarnessAccessMode::Full]
    }

    fn confirm(
        &self,
        process: &mut PiRpcProcess,
        control: &str,
        mode: HarnessAccessMode,
    ) -> Result<(), String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let expected = mode_name(mode)?;
        let request = json!({"requestId": request_id, "files": expected, "network": expected});
        let command = json!({"type": "prompt", "message": format!("/{control} {request}")});
        // get_commands established this as an extension command before we send it.
        // It must complete locally and never start an agent turn.
        process.confirm_control(command, Duration::from_secs(15), |event| match event {
            SessionEvent::Interaction(ExtensionUiRequest::SetStatus { key, text, .. })
                if key == STATUS_KEY =>
            {
                Some(confirm_result(text.as_deref(), &request_id, expected))
            }
            SessionEvent::Activity(activity)
                if matches!(
                    activity.kind(),
                    crate::agents::SessionActivityKind::AgentStarted
                ) =>
            {
                Some(Err(
                    "pi-nono control unexpectedly started an agent turn".into()
                ))
            }
            _ => None,
        })
    }

    fn mode_report(
        &self,
        request: &ExtensionUiRequest,
    ) -> Option<Result<HarnessAccessMode, String>> {
        let ExtensionUiRequest::SetStatus { key, text, .. } = request else {
            return None;
        };
        if key != STATUS_KEY {
            return None;
        }
        Some((|| {
            let result = decode_result(text.as_deref())?;
            match (result.files.as_str(), result.network.as_str()) {
                ("sandboxed", "sandboxed") => Ok(HarnessAccessMode::Sandboxed),
                ("full", "full") => Ok(HarnessAccessMode::Full),
                _ => Err("pi-nono reported an unsupported sandbox mode".into()),
            }
        })())
    }
}

fn mode_name(mode: HarnessAccessMode) -> Result<&'static str, String> {
    match mode {
        HarnessAccessMode::Sandboxed => Ok("sandboxed"),
        HarnessAccessMode::Full => Ok("full"),
        HarnessAccessMode::Auto => {
            Err("pi-nono does not support automatic sandbox approvals".into())
        }
    }
}

fn control_command(commands: &[PiCommand]) -> Result<Option<&str>, String> {
    let mut controls = commands.iter().filter(|command| {
        let name = command.name.as_str();
        let is_control = name == "sandbox-mode"
            || name.strip_prefix("sandbox-mode:").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            });
        is_control
            && command.source == SlashCommandSource::Extension
            && command
                .source_info
                .as_ref()
                .and_then(|source| source.path.as_deref())
                .is_some_and(is_nono_source)
    });
    let control = controls.next();
    if controls.next().is_some() {
        return Err("Multiple pi-nono sandbox controls loaded; cannot choose safely".into());
    }
    Ok(control.map(|command| command.name.as_str()))
}

fn is_nono_source(path: &Path) -> bool {
    // Inspect only the owner of a command Pi actually loaded, never scan extension
    // folders or infer identity from a command name or directory called sandbox.
    if !path.is_absolute() {
        return false;
    }
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    for directory in path.ancestors().skip(1).take(8) {
        let manifest = directory.join("package.json");
        let metadata = match manifest.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return false,
        };
        if !metadata.is_file() || metadata.len() > 64 * 1024 {
            return false;
        }
        let Ok(file) = std::fs::File::open(manifest) else {
            return false;
        };
        let mut bytes = Vec::new();
        if file.take(64 * 1024).read_to_end(&mut bytes).is_err() {
            return false;
        }
        return serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|package| {
                package.get("name").and_then(|name| name.as_str()) == Some("pi-nono")
            });
    }
    false
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModeResult {
    version: u32,
    request_id: String,
    files: String,
    network: String,
    success: bool,
    error: Option<String>,
}

fn confirm_result(text: Option<&str>, request_id: &str, expected: &str) -> Result<(), String> {
    let result = decode_result(text)?;
    if result.request_id != request_id {
        return Err("Stale pi-nono sandbox confirmation".into());
    }
    if result.files != expected || result.network != expected {
        return Err("pi-nono did not confirm the requested filesystem and network mode".into());
    }
    Ok(())
}

fn decode_result(text: Option<&str>) -> Result<ModeResult, String> {
    let result: ModeResult = serde_json::from_str(text.unwrap_or_default())
        .map_err(|error| format!("Invalid pi-nono sandbox confirmation: {error}"))?;
    if result.version != 1 {
        return Err("Unsupported pi-nono sandbox protocol version".into());
    }
    if !result.success {
        return Err(format!(
            "pi-nono sandbox is unavailable: {}",
            result.error.as_deref().unwrap_or("mode change failed")
        ));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "nono_tests.rs"]
mod tests;
