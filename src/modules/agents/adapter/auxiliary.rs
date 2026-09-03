use std::{
    path::Path,
    process::Stdio,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::agents::{
    AgentLaunchConfig, ConfigurationCatalog, WorkerContext, WorkerEvent, WorkerLaunch,
    WorkerSendMode, WorkerSessionFactory,
};

const TITLE_TIMEOUT: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn supports_auto_title_generation(harness: &str) -> bool {
    matches!(harness, "pi" | "codex-cli")
}

pub(crate) fn generate_session_title(
    config: &AgentLaunchConfig,
    harness: &str,
    project: &Path,
    first_prompt: &str,
    active_model: Option<&crate::protocol::Model>,
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
    let output = match harness {
        // Pi title inference must never enter the transcript-oriented RPC worker path.
        "pi" => generate_pi_title(
            config,
            project,
            first_prompt,
            selection.as_ref(),
            effort.as_deref(),
        ),
        "codex-cli" => generate_worker_title(
            config,
            harness,
            project,
            first_prompt,
            selection.as_ref(),
            effort,
        ),
        _ => unreachable!("unsupported title backend was rejected above"),
    }?;
    normalize_title(&output)
}

fn generate_worker_title(
    config: &AgentLaunchConfig,
    harness: &str,
    project: &Path,
    first_prompt: &str,
    selection: Option<&crate::protocol::Model>,
    effort: Option<String>,
) -> Result<String, String> {
    let factory = title_factory(config, harness)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut session = factory.create(WorkerLaunch {
        worker_id: format!("title-{nonce}"),
        worker_name: "session-title".into(),
        project: project.to_owned(),
        parent_session: "ephemeral".into(),
        parent_worker_id: None,
        context: WorkerContext::Fresh,
        provider: selection.map(|model| model.provider.clone()),
        model: selection.map(|model| model.id.clone()),
        effort,
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

fn generate_pi_title(
    config: &AgentLaunchConfig,
    project: &Path,
    first_prompt: &str,
    selection: Option<&crate::protocol::Model>,
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
    selection: Option<&crate::protocol::Model>,
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
    harness: &str,
) -> Result<Arc<dyn WorkerSessionFactory>, String> {
    let (factories, _) = super::worker_factories(config.clone());
    factories
        .get(harness)
        .cloned()
        .ok_or_else(|| format!("unsupported title generator backend: {harness}"))
}

fn title_model(
    harness: &str,
    catalog: &ConfigurationCatalog,
    active_model: Option<&crate::protocol::Model>,
) -> Option<crate::protocol::Model> {
    let override_name = match harness {
        "pi" => "FARCASTER_PI_TITLE_MODEL",
        "codex-cli" => "FARCASTER_CODEX_TITLE_MODEL",
        _ => return None,
    };
    if let Some(requested) =
        std::env::var_os(override_name).and_then(|value| value.into_string().ok())
    {
        if let Some(model) = catalog.models.iter().find(|model| {
            model.id == requested || format!("{}/{}", model.provider, model.id) == requested
        }) {
            return Some(model.clone());
        }
    }
    let preferences: &[&str] = match harness {
        "pi" => match active_model.map(|model| model.provider.as_str()) {
            Some("openai-codex" | "openai") => &["gpt-5.6-luna", "luna", "nano", "mini"],
            Some("anthropic") => &["haiku"],
            Some("google") => &["flash-lite", "flash"],
            Some(_) => &["nano", "mini", "small", "lite", "flash"],
            None => &[],
        },
        "codex-cli" => &["gpt-5.6-luna", "luna", "nano", "mini"],
        _ => &[],
    };
    let selected = catalog
        .models
        .iter()
        .filter(|model| {
            harness != "pi" || active_model.is_some_and(|active| model.provider == active.provider)
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
    selected.or_else(|| (harness == "pi").then(|| active_model.cloned()).flatten())
}

fn lowest_effort(
    catalog: &ConfigurationCatalog,
    model: Option<&crate::protocol::Model>,
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
mod tests {
    use super::*;
    use crate::protocol::Model;

    fn model(id: &str, reasoning: bool) -> Model {
        model_from("provider", id, reasoning)
    }

    fn model_from(provider: &str, id: &str, reasoning: bool) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            provider: provider.into(),
            context_window: 0,
            reasoning,
            efforts: None,
        }
    }

    #[test]
    fn pi_title_command_is_isolated_from_session_transport() {
        let project = std::env::current_dir().unwrap();
        let command = pi_title_command(
            &AgentLaunchConfig::default(),
            &project,
            "Fix the transcript",
            None,
            Some("low"),
        )
        .unwrap();
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        for required in [
            "--print",
            "--no-session",
            "--no-tools",
            "--no-extensions",
            "--no-context-files",
        ] {
            assert!(arguments.iter().any(|argument| argument == required));
        }
        assert!(!arguments.iter().any(|argument| argument == "--mode"));
        assert_eq!(
            arguments.last().map(String::as_str),
            Some("Fix the transcript")
        );
    }

    #[test]
    fn pi_prefers_a_cheap_model_from_the_active_provider() {
        let catalog = ConfigurationCatalog {
            models: vec![
                model_from("google", "gemini-flash-lite", false),
                model_from("anthropic", "claude-opus", true),
                model_from("anthropic", "claude-haiku", true),
            ],
            efforts: Vec::new(),
        };
        let active = model_from("anthropic", "claude-opus", true);
        let selected = title_model("pi", &catalog, Some(&active)).unwrap();
        assert_eq!(selected.provider, "anthropic");
        assert_eq!(selected.id, "claude-haiku");
    }

    #[test]
    fn pi_ignores_image_models_with_cheap_display_names() {
        let mut image = model_from("google", "gemini-3.1-flash-lite-image", true);
        image.name = "Nano Banana 2 Lite".into();
        let catalog = ConfigurationCatalog {
            models: vec![image, model_from("google", "gemini-2.5-flash-lite", true)],
            efforts: Vec::new(),
        };
        let active = model_from("google", "gemini-2.5-pro", true);
        assert_eq!(
            title_model("pi", &catalog, Some(&active)).unwrap().id,
            "gemini-2.5-flash-lite"
        );
    }

    #[test]
    fn pi_without_an_active_model_uses_backend_default() {
        let catalog = ConfigurationCatalog {
            models: vec![model_from("google", "gemini-flash-lite", false)],
            efforts: Vec::new(),
        };
        assert_eq!(title_model("pi", &catalog, None), None);
    }

    #[test]
    fn pi_falls_back_to_the_active_model() {
        let active = model_from("custom", "custom-large", true);
        assert_eq!(
            title_model("pi", &ConfigurationCatalog::default(), Some(&active)),
            Some(active)
        );
    }

    #[test]
    fn codex_prefers_luna() {
        let catalog = ConfigurationCatalog {
            models: vec![
                model_from("openai", "gpt-5.4-mini", true),
                model_from("openai", "gpt-5.6-luna", true),
            ],
            efforts: Vec::new(),
        };
        assert_eq!(
            title_model("codex-cli", &catalog, None).unwrap().id,
            "gpt-5.6-luna"
        );
    }

    #[test]
    fn no_known_cheap_model_uses_backend_default() {
        let catalog = ConfigurationCatalog {
            models: vec![model("custom", false)],
            efforts: Vec::new(),
        };
        assert_eq!(title_model("codex-cli", &catalog, None), None);
    }

    #[test]
    fn uses_lowest_advertised_reasoning_effort() {
        let mut selected = model("reasoning", true);
        selected.efforts = Some(vec!["high".into(), "minimal".into(), "off".into()]);
        assert_eq!(
            lowest_effort(&ConfigurationCatalog::default(), Some(&selected)),
            Some("off".into())
        );
    }

    #[test]
    fn normalizes_model_title_output() {
        assert_eq!(
            normalize_title("**Title: Fix session names.**\n").unwrap(),
            "Fix session names"
        );
    }
}
