use super::*;
use serde_json::json;

#[test]
fn command_preview_preserves_source_and_ignores_missing_or_invalid_commands() {
    let command = "cargo test mcp\ngit diff --check";
    for (arguments, expected) in [
        (json!({"command": command}), Some(command)),
        (json!({}), None),
        (json!({"command": " \n"}), None),
        (json!({"command": []}), None),
    ] {
        let details = ToolDetails::from_call("bash", Some(&arguments), None);
        assert_eq!(details.command_preview(), expected);
    }
}

#[test]
fn readable_command_leaves_the_exact_invocation_in_inspection() {
    let script = "printf '%s\\n' \"$HOME\"\ngit diff --check";
    let command = shell_words::join(["/run/current-system/sw/bin/zsh", "-lc", script]);
    let arguments = json!({"command": command});
    let details = ToolDetails::from_call("bash", Some(&arguments), None);
    assert_eq!(details.display_command().as_deref(), Some(script));
    assert_eq!(details.command_preview(), Some(command.as_str()));
    assert_eq!(details.summary(), "printf '%s\\n' \"$HOME\"…");
    assert_eq!(details.arguments, arguments);
    assert!(
        details
            .inspection_text()
            .contains(&serde_json::to_string(&command).unwrap())
    );
}
