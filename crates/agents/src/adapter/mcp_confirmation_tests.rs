use super::*;

#[test]
fn prompt_preserves_message_and_action_without_internal_session_metadata() {
    let prompt = prompt(
        "node_repl",
        "Allow Computer Use to use Farcaster?",
        "form",
        &json!({"type":"object","properties":{}}),
        &json!({"subtitle":"App access", "tool_name":"get_app_state", "riskLevel":"low",
            "tool_params":{"app":"io.github.behzade.farcaster.dev"},
            "x-codex-turn-metadata":{"session_id":"internal-session"}}),
    )
    .expect("confirmation");
    for text in [
        "Allow Computer Use to use Farcaster?",
        "node_repl",
        "App access",
        "get_app_state",
        "low",
        "io.github.behzade.farcaster.dev",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    assert!(!prompt.contains("internal-session"));
}

#[test]
fn confirmation_does_not_accept_forms_that_need_values_or_other_constraints() {
    for schema in [
        Value::Null,
        json!({}),
        json!({"type":"string"}),
        json!({"type":"object","properties":{"name":{"type":"string"}}}),
        json!({"type":"object","required":["name"]}),
        json!({"type":"object","minProperties":1}),
        json!({"type":"object","not":{}}),
        json!({"type":"object","properties":[]}),
        json!({"type":"object","additionalProperties":{"type":"string"}}),
    ] {
        assert!(
            prompt("server", "Confirm?", "form", &schema, &Value::Null).is_err(),
            "{schema}"
        );
    }
    for mode in ["url", "openai/form", ""] {
        assert!(
            prompt(
                "server",
                "Confirm?",
                mode,
                &json!({"type":"object"}),
                &Value::Null
            )
            .is_err()
        );
    }
}
