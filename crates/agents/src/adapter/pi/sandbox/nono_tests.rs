use super::*;

#[test]
fn sandbox_discovery_requires_a_loaded_pi_nono_source() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("index.js");
    std::fs::write(&source, "")?;
    let manifest = temp.path().join("package.json");
    std::fs::write(&manifest, r#"{"name":"pi-nono"}"#)?;
    let mut command: PiCommand = serde_json::from_value(json!({
        "name": "sandbox-mode", "source": "extension", "sourceInfo": {"path": source}
    }))?;
    for source in [
        SlashCommandSource::Prompt,
        SlashCommandSource::Skill,
        SlashCommandSource::Extension,
    ] {
        command.source = source;
        assert_eq!(
            control_command(std::slice::from_ref(&command))?.is_some(),
            source == SlashCommandSource::Extension
        );
    }
    command.name = "sandbox-mode:2".into();
    assert_eq!(control_command(&[command.clone()])?, Some("sandbox-mode:2"));
    assert!(control_command(&[command.clone(), command.clone()]).is_err());
    std::fs::write(&manifest, r#"{"name":"unrelated"}"#)?;
    assert!(control_command(&[command.clone()])?.is_none());
    std::fs::write(&manifest, "invalid")?;
    assert!(control_command(&[command.clone()])?.is_none());
    command.source_info = None;
    assert!(control_command(&[command])?.is_none());
    Ok(())
}

#[test]
fn sandbox_confirmation_requires_matching_success_version_and_both_access_modes() {
    let valid = json!({"version":1,"requestId":"current","files":"sandboxed","network":"sandboxed","success":true});
    assert!(confirm_result(Some(&valid.to_string()), "current", "sandboxed").is_ok());
    for (field, value) in [
        ("version", json!(2)),
        ("requestId", json!("old")),
        ("files", json!("full")),
        ("network", json!("full")),
        ("success", json!(false)),
    ] {
        let mut response = valid.clone();
        response[field] = value;
        assert!(
            confirm_result(Some(&response.to_string()), "current", "sandboxed").is_err(),
            "{field}"
        );
    }
    assert!(confirm_result(None, "current", "sandboxed").is_err());
    assert!(confirm_result(Some("not json"), "current", "sandboxed").is_err());
}
