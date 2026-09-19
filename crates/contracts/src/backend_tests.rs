use super::Backend;

#[test]
fn backend_ids_are_stable_machine_keys() -> Result<(), String> {
    assert_eq!("codex-cli".parse::<Backend>()?.as_str(), "codex-cli");
    assert!("Codex CLI".parse::<Backend>().is_err());
    assert!("".parse::<Backend>().is_err());
    Ok(())
}

#[test]
fn backend_round_trips_preserve_storage_and_wire_names() {
    let expected = [
        "pi",
        "codex-cli",
        "cursor-cli",
        "opencode",
        "claude",
        "antigravity-acp",
    ];
    for (backend, name) in Backend::ALL.into_iter().zip(expected) {
        assert_eq!(backend.as_str(), name);
        assert_eq!(name.parse::<Backend>(), Ok(backend));
        assert_eq!(
            serde_json::to_value(backend).expect("serialize backend"),
            name
        );
        assert_eq!(
            serde_json::from_value::<Backend>(serde_json::json!(name))
                .expect("deserialize backend"),
            backend
        );
    }
    for name in ["", "unknown", "Pi", "codex"] {
        assert!(name.parse::<Backend>().is_err());
        assert!(serde_json::from_value::<Backend>(serde_json::json!(name)).is_err());
    }
    let legacy: Backend = serde_json::from_str(r#""opencode2""#).expect("legacy backend alias");
    assert_eq!(legacy, Backend::OpenCode);
    assert_eq!(
        serde_json::to_string(&legacy).expect("serialize backend"),
        r#""opencode""#
    );
}
