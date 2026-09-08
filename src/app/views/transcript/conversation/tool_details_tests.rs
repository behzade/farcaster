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
