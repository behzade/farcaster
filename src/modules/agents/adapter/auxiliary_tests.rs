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
    let project = std::env::current_dir().expect("current project directory");
    let command = pi_title_command(
        &AgentLaunchConfig::default(),
        &project,
        "Fix the transcript",
        None,
        Some("low"),
    )
    .expect("build isolated Pi title command");
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
    let selected =
        title_model("pi", &catalog, Some(&active)).expect("cheap model from active provider");
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
        title_model("pi", &catalog, Some(&active))
            .expect("non-image cheap model")
            .id,
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
        title_model("codex-cli", &catalog, None)
            .expect("preferred Codex model")
            .id,
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
        normalize_title("**Title: Fix session names.**\n").expect("normalized title"),
        "Fix session names"
    );
}
