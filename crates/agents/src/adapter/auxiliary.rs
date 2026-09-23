use crate::Backend;
use std::{
    path::Path,
    process::Stdio,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    AgentLaunchConfig, ConfigurationCatalog, WorkerContext, WorkerEvent, WorkerLaunch,
    WorkerSendMode, WorkerSessionFactory,
};

const TITLE_TIMEOUT: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn supports_auto_title_generation(harness: impl Into<Option<Backend>>) -> bool {
    let Some(harness) = harness.into() else {
        return false;
    };
    super::backend::for_backend(harness).supports_auto_title_generation()
}

pub fn generate_session_title(
    config: &AgentLaunchConfig,
    harness: Backend,
    project: &Path,
    first_prompt: &str,
    active_model: Option<&crate::extensions::Model>,
) -> Result<String, String> {
    if !supports_auto_title_generation(harness) {
        return Err(format!("{harness} does not expose ephemeral inference"));
    }
    let catalog = super::load_configuration_catalog(config, harness, project).unwrap_or_default();
    let selection = title_model(harness, &catalog, active_model);
    let effort = lowest_effort(&catalog, selection.as_ref());
    let selected = selection
        .as_ref()
        .map(|model| format!("{}/{}", model.provider, model.id))
        .unwrap_or_else(|| "backend-default".into());
    zlog::info!("Generating {harness} session title with {selected}");
    let output = super::backend::for_backend(harness).generate_title(
        config,
        project,
        first_prompt,
        selection.as_ref(),
        effort,
    )?;
    normalize_title(&output)
}

pub(super) fn generate_worker_title(
    config: &AgentLaunchConfig,
    harness: Backend,
    project: &Path,
    first_prompt: &str,
    selection: Option<&crate::extensions::Model>,
    effort: Option<String>,
) -> Result<String, String> {
    let factory = title_factory(config, harness)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut session = factory.create(WorkerLaunch {
        slot: None,
        worker_id: format!("title-{nonce}"),
        worker_name: "session-title".into(),
        project: project.to_owned(),
        parent_session: "ephemeral".into(),
        parent_worker_id: None,
        context: WorkerContext::Fresh,
        provider: selection.map(|model| model.provider.clone()),
        model: selection.map(|model| model.id.clone()),
        effort,
        service_tier: None,
        access_mode: config.access_mode,
        app_proxy: config.app_proxy.clone(),
        ephemeral: true,
    })?;
    if let Err(error) = session.send(title_prompt(first_prompt), WorkerSendMode::Prompt) {
        let _ = session.close();
        return Err(error);
    }
    let deadline = Instant::now() + TITLE_TIMEOUT;
    let result = loop {
        if Instant::now() >= deadline {
            let _ = session.abort();
            break Err("session title generation timed out".into());
        }
        match session.poll() {
            Some(WorkerEvent::Settled { output }) => break Ok(output),
            Some(WorkerEvent::Failed(error)) => break Err(error),
            Some(WorkerEvent::NeedsInput(_)) => {
                let _ = session.abort();
                break Err("session title generation requested user input".into());
            }
            Some(_) => {}
            None => thread::sleep(POLL_INTERVAL),
        }
    };
    let close = session.close();
    match (result, close) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

pub(super) fn generate_pi_title(
    config: &AgentLaunchConfig,
    project: &Path,
    first_prompt: &str,
    selection: Option<&crate::extensions::Model>,
    effort: Option<&str>,
) -> Result<String, String> {
    let mut command = pi_title_command(config, project, first_prompt, selection, effort)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("start ephemeral Pi title generator: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Pi title stdout was not piped".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Pi title stderr was not piped".to_owned())?;
    let stdout = thread::spawn(move || read_output(stdout));
    let stderr = thread::spawn(move || read_output(stderr));
    let deadline = Instant::now() + TITLE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("session title generation timed out".to_owned());
            }
            Err(error) => break Err(format!("wait for Pi title generator: {error}")),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let status = status?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "Pi title generator exited {}: {}",
            status.code().unwrap_or(-1),
            stderr.trim().chars().take(500).collect::<String>()
        ));
    }
    String::from_utf8(stdout).map_err(|error| format!("decode Pi title output: {error}"))
}

fn pi_title_command(
    config: &AgentLaunchConfig,
    project: &Path,
    first_prompt: &str,
    selection: Option<&crate::extensions::Model>,
    effort: Option<&str>,
) -> Result<std::process::Command, String> {
    let mut command = config.command(project)?;
    command.args([
        "--print",
        "--no-session",
        "--no-tools",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-context-files",
        "--no-approve",
        "--system-prompt",
        "Create a concise coding-session title. Output only the title: 3-8 words, at most 60 characters, without quotes, markdown, a label, or final punctuation.",
    ]);
    if let Some(model) = selection {
        command.args(["--provider", &model.provider, "--model", &model.id]);
    }
    if let Some(effort) = effort {
        command.args(["--thinking", effort]);
    }
    command.arg(first_prompt.chars().take(8_000).collect::<String>());
    Ok(command)
}

fn read_output(mut reader: impl std::io::Read) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = reader.read_to_end(&mut output);
    output
}

fn title_factory(
    config: &AgentLaunchConfig,
    harness: Backend,
) -> Result<Arc<dyn WorkerSessionFactory>, String> {
    let (factories, _) = super::worker_factories(config.clone());
    factories
        .get(&harness)
        .cloned()
        .ok_or_else(|| format!("unsupported title generator backend: {harness}"))
}

fn title_model(
    harness: Backend,
    catalog: &ConfigurationCatalog,
    active_model: Option<&crate::extensions::Model>,
) -> Option<crate::extensions::Model> {
    super::backend::for_backend(harness).title_model(catalog, active_model)
}

pub(super) fn select_title_model(
    catalog: &ConfigurationCatalog,
    active_model: Option<&crate::extensions::Model>,
    override_name: &str,
    preferences: &[&str],
    same_provider: bool,
) -> Option<crate::extensions::Model> {
    if let Some(requested) =
        std::env::var_os(override_name).and_then(|value| value.into_string().ok())
        && let Some(model) = catalog.models.iter().find(|model| {
            model.id == requested || format!("{}/{}", model.provider, model.id) == requested
        })
    {
        return Some(model.clone());
    }
    let selected = catalog
        .models
        .iter()
        .filter(|model| {
            !same_provider || active_model.is_some_and(|active| model.provider == active.provider)
        })
        .filter(|model| {
            let id = model.id.to_ascii_lowercase();
            !["image", "vision", "live", "computer-use", "deep-research"]
                .iter()
                .any(|excluded| id.contains(excluded))
        })
        .filter_map(|model| {
            let id = model.id.to_ascii_lowercase();
            let rank = preferences
                .iter()
                .position(|candidate| id.contains(candidate))?;
            Some(((rank, model.reasoning, model.id.as_str()), model))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, model)| model.clone());
    selected.or_else(|| same_provider.then(|| active_model.cloned()).flatten())
}

fn lowest_effort(
    catalog: &ConfigurationCatalog,
    model: Option<&crate::extensions::Model>,
) -> Option<String> {
    if model.is_some_and(|model| !model.reasoning) {
        return None;
    }
    let efforts = model
        .and_then(|model| model.efforts.as_ref())
        .unwrap_or(&catalog.efforts);
    ["off", "none", "minimal", "low"]
        .into_iter()
        .find_map(|candidate| {
            efforts
                .iter()
                .find(|effort| effort.eq_ignore_ascii_case(candidate))
                .cloned()
        })
}

fn title_prompt(first_prompt: &str) -> String {
    let prompt = first_prompt.chars().take(8_000).collect::<String>();
    format!(
        "Write a concise title for the following coding-agent session. Do not use tools or inspect the project. Return only the title, with no quotes, markdown, label, or final punctuation. Use 3-8 words and at most 60 characters.\n\n{prompt}"
    )
}

fn normalize_title(output: &str) -> Result<String, String> {
    let line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '`' | '*' | '#'))
        .trim()
        .strip_prefix("Title:")
        .unwrap_or_else(|| {
            output
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default()
                .trim()
        })
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '`' | '*' | '#'))
        .trim_end_matches(['.', ':', ';'])
        .trim();
    let title = line
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect::<String>();
    let title = title.trim();
    if title.is_empty() {
        Err("session title generator returned an empty title".into())
    } else {
        Ok(title.to_owned())
    }
}

#[cfg(test)]
#[path = "auxiliary_tests.rs"]
mod tests;
