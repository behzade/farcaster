use std::{
    path::Path,
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
) -> Result<String, String> {
    if !supports_auto_title_generation(harness) {
        return Err(format!("{harness} does not expose ephemeral inference"));
    }
    let catalog = super::load_configuration_catalog(config, harness, project).unwrap_or_default();
    let selection = title_model(harness, &catalog);
    let effort = lowest_effort(&catalog, selection.as_ref());
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
        provider: selection.as_ref().map(|model| model.provider.clone()),
        model: selection.map(|model| model.id),
        effort,
        ephemeral: true,
    })?;
    let prompt = title_prompt(first_prompt);
    if let Err(error) = session.send(prompt, WorkerSendMode::Prompt) {
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
            Some(WorkerEvent::Settled { output }) => break normalize_title(&output),
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
        (Ok(title), Ok(())) => Ok(title),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
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

fn title_model(harness: &str, catalog: &ConfigurationCatalog) -> Option<crate::protocol::Model> {
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
        "pi" => &[
            "nano",
            "flash-lite",
            "haiku",
            "mini",
            "flash",
            "small",
            "lite",
        ],
        "codex-cli" => &["nano", "mini"],
        _ => &[],
    };
    catalog
        .models
        .iter()
        .filter_map(|model| {
            let identity = format!("{} {}", model.id, model.name).to_ascii_lowercase();
            let rank = preferences
                .iter()
                .position(|candidate| identity.contains(candidate))?;
            Some(((rank, model.reasoning, model.id.as_str()), model))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, model)| model.clone())
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
    ["none", "minimal", "low"]
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
        Model {
            id: id.into(),
            name: id.into(),
            provider: "provider".into(),
            context_window: 0,
            reasoning,
            efforts: None,
        }
    }

    #[test]
    fn pi_prefers_a_cheap_advertised_model() {
        let catalog = ConfigurationCatalog {
            models: vec![
                model("large", false),
                model("flash", false),
                model("haiku", false),
            ],
            efforts: Vec::new(),
        };
        assert_eq!(title_model("pi", &catalog).unwrap().id, "haiku");
    }

    #[test]
    fn no_known_cheap_model_uses_backend_default() {
        let catalog = ConfigurationCatalog {
            models: vec![model("custom", false)],
            efforts: Vec::new(),
        };
        assert_eq!(title_model("codex-cli", &catalog), None);
    }

    #[test]
    fn normalizes_model_title_output() {
        assert_eq!(
            normalize_title("**Title: Fix session names.**\n").unwrap(),
            "Fix session names"
        );
    }
}
