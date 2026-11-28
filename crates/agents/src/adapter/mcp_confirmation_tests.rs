use super::*;

#[test]
fn prompt_shows_readable_details_and_keeps_the_native_request_internally() {
    let request = json!({
        "message":"Allow Computer Use to use Farcaster?", "mode":"form",
        "requestedSchema":{"type":"object","properties":{}},
        "_meta":{"connector_name":"Computer Use", "subtitle":"App access", "tool_name":"get_app_state", "riskLevel":"low",
            "tool_params":{"app":"io.github.behzade.farcaster.dev"},
            "tool_params_display":[{"name":"app","display_name":"App","value":"Farcaster"}],
            "persist":["session","always"],"unknown_extension":{"keep":true}}
    });
    let confirmation = Confirmation::parse("node_repl", &request).expect("confirmation");
    assert_eq!(confirmation.request, request);
    assert_eq!(
        confirmation.prompt,
        "Allow Computer Use to use Farcaster?\nRequested by: Computer Use\nNote: App access\nAction: get_app_state\nRisk: low\nApp: Farcaster"
    );
}

#[test]
fn persistence_choices_require_native_support_and_explicit_user_selection() {
    for (persist, allow, persistence, always) in [
        (Value::Null, "Allow", None, false),
        (
            json!(["session"]),
            "Allow for this session",
            Some("session"),
            false,
        ),
        (
            json!(["session", "always"]),
            "Allow for this session",
            Some("session"),
            true,
        ),
        (json!(["always"]), "Allow", None, true),
        (json!(["unknown"]), "Allow", None, false),
        (json!("always"), "Allow", None, false),
    ] {
        let request = json!({"message":"Confirm?", "mode":"form", "requestedSchema":{"type":"object"}, "_meta":{"persist":persist}});
        let confirmation = Confirmation::parse("server", &request).expect("confirmation");
        let mut options = vec!["Decline", allow];
        if always {
            options.push("Always allow");
        }
        assert_eq!(confirmation.options(), options);
        let mut accepted = json!({"action":"accept","content":{}});
        if let Some(persistence) = persistence {
            accepted["_meta"] = json!({"persist":persistence});
        }
        assert_eq!(confirmation.reply(Some(allow), false), accepted);
        assert_eq!(
            confirmation.reply(Some("Always allow"), false),
            if always {
                json!({"action":"accept","content":{},"_meta":{"persist":"always"}})
            } else {
                json!({"action":"decline","content":null})
            }
        );
        for choice in [None, Some("Decline"), Some("unknown")] {
            assert_eq!(
                confirmation.reply(choice, false),
                json!({"action":"decline","content":null})
            );
        }
        assert_eq!(
            confirmation.reply(Some("Always allow"), true),
            json!({"action":"cancel","content":null})
        );
    }
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
            Confirmation::parse(
                "server",
                &json!({"message":"Confirm?", "mode":"form", "requestedSchema":schema})
            )
            .is_err(),
            "{schema}"
        );
    }
    for mode in ["url", "openai/form", ""] {
        assert!(
            Confirmation::parse(
                "server",
                &json!({"message":"Confirm?", "mode":mode,"requestedSchema":{"type":"object"}})
            )
            .is_err()
        );
    }
}
