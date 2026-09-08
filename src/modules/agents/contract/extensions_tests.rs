use super::*;

#[test]
fn model_efforts_distinguish_unknown_from_known_empty() {
    let legacy = serde_json::from_value::<Model>(json!({
        "id": "legacy",
        "name": "Legacy",
        "provider": "provider",
        "reasoning": true
    }))
    .expect("legacy model");
    let known = serde_json::from_value::<Model>(json!({
        "id": "known",
        "name": "Known",
        "provider": "provider",
        "reasoning": true,
        "efforts": []
    }))
    .expect("known model");

    assert_eq!(legacy.efforts, None);
    assert_eq!(known.efforts, Some(Vec::new()));
}

#[test]
fn gpui_notification_transport_separates_title_and_body() {
    let request = ExtensionUiRequest::Notify {
        id: "notification".into(),
        message: "\u{1f}farcaster-notification\u{1f}Pi finished\u{1f}Done".into(),
        tone: NotifyTone::Info,
    };
    assert_eq!(
        request.gpui_system_notification(),
        Some(("Pi finished", "Done"))
    );
    let legacy = ExtensionUiRequest::Notify {
        id: "notification".into(),
        message: "\u{1f}pi-gpui-notification\u{1f}Pi finished\u{1f}Done".into(),
        tone: NotifyTone::Info,
    };
    assert_eq!(
        legacy.gpui_system_notification(),
        Some(("Pi finished", "Done"))
    );
}

#[test]
fn decodes_every_extension_ui_request_exactly() {
    let cases = [
        (
            r#"{"type":"extension_ui_request","id":"1","method":"select","title":"T","options":["a"],"timeout":10}"#,
            ExtensionUiRequest::Select {
                id: "1".into(),
                title: "T".into(),
                options: vec!["a".into()],
                timeout: Some(10),
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"2","method":"confirm","title":"T","message":"M","timeout":11}"#,
            ExtensionUiRequest::Confirm {
                id: "2".into(),
                title: "T".into(),
                message: "M".into(),
                timeout: Some(11),
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"3","method":"input","title":"T","placeholder":"P","timeout":12}"#,
            ExtensionUiRequest::Input {
                id: "3".into(),
                title: "T".into(),
                placeholder: Some("P".into()),
                timeout: Some(12),
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"4","method":"editor","title":"T","prefill":"P"}"#,
            ExtensionUiRequest::Editor {
                id: "4".into(),
                title: "T".into(),
                prefill: Some("P".into()),
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"5","method":"notify","message":"M","notifyType":"error"}"#,
            ExtensionUiRequest::Notify {
                id: "5".into(),
                message: "M".into(),
                tone: NotifyTone::Error,
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"6","method":"setStatus","statusKey":"k"}"#,
            ExtensionUiRequest::SetStatus {
                id: "6".into(),
                key: "k".into(),
                text: None,
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"7","method":"setWidget","widgetKey":"k","widgetLines":["x"],"widgetPlacement":"belowEditor"}"#,
            ExtensionUiRequest::SetWidget {
                id: "7".into(),
                key: "k".into(),
                lines: Some(vec!["x".into()]),
                placement: WidgetPlacement::BelowEditor,
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"8","method":"setTitle","title":"T"}"#,
            ExtensionUiRequest::SetTitle {
                id: "8".into(),
                title: "T".into(),
            },
        ),
        (
            r#"{"type":"extension_ui_request","id":"9","method":"set_editor_text","text":"x"}"#,
            ExtensionUiRequest::SetEditorText {
                id: "9".into(),
                text: "x".into(),
            },
        ),
    ];
    for (frame, expected) in cases {
        assert_eq!(
            serde_json::from_str::<ExtensionUiRequest>(frame).expect("extension request"),
            expected
        );
    }
}

#[test]
fn rejects_malformed_known_extension_requests() {
    for frame in [
        r#"{"type":"extension_ui_request","method":"confirm","title":"T","message":"M"}"#,
        r#"{"type":"extension_ui_request","id":"1","method":"select","title":"T","options":["a",1]}"#,
    ] {
        assert!(
            serde_json::from_str::<ExtensionUiRequest>(frame).is_err(),
            "accepted {frame}"
        );
    }
}

#[test]
fn unknown_extension_enum_values_use_protocol_defaults() {
    assert_eq!(
            serde_json::from_str::<ExtensionUiRequest>(
                r#"{"type":"extension_ui_request","id":"1","method":"notify","message":"M","notifyType":"future"}"#
            ).expect("notification"),
            ExtensionUiRequest::Notify {
                id: "1".into(),
                message: "M".into(),
                tone: NotifyTone::Info,
            }
        );
    assert_eq!(
            serde_json::from_str::<ExtensionUiRequest>(
                r#"{"type":"extension_ui_request","id":"2","method":"setWidget","widgetKey":"k","widgetPlacement":"future"}"#
            ).expect("widget"),
            ExtensionUiRequest::SetWidget {
                id: "2".into(),
                key: "k".into(),
                lines: None,
                placement: WidgetPlacement::AboveEditor,
            }
        );
}

#[test]
fn serializes_all_response_shapes() -> Result<(), serde_json::Error> {
    let values = [
        ExtensionUiResponse::Value {
            id: "1".into(),
            value: "a".into(),
        },
        ExtensionUiResponse::Confirmed {
            id: "2".into(),
            confirmed: true,
        },
        ExtensionUiResponse::Cancelled {
            id: "3".into(),
            cancelled: true,
        },
    ];
    let encoded = values
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        encoded[0],
        json!({"type":"extension_ui_response","id":"1","value":"a"})
    );
    assert_eq!(
        encoded[1],
        json!({"type":"extension_ui_response","id":"2","confirmed":true})
    );
    assert_eq!(
        encoded[2],
        json!({"type":"extension_ui_response","id":"3","cancelled":true})
    );
    Ok(())
}

#[test]
fn slash_commands_decode_without_depending_on_source_metadata_shape() {
    let command = serde_json::from_value::<SlashCommand>(json!({
        "name": "reload",
        "description": "Reload extensions",
        "source": "extension",
        "sourceInfo": {"scope": "project"}
    }))
    .expect("slash command should decode");
    assert_eq!(
        command,
        SlashCommand {
            name: "reload".into(),
            description: Some("Reload extensions".into()),
            source: SlashCommandSource::Extension,
        }
    );
}
