use super::*;
use crate::conversation::ConversationState;
use serde_json::json;

fn item(arguments: Value) -> TranscriptItem {
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type": "tool_execution_start", "toolCallId": "test",
        "toolName": "worker_notices", "args": arguments
    }));
    (*state.items[0]).clone()
}

#[test]
fn structured_input_keeps_prose_literal_and_paths_distinct() {
    let item = item(json!({
        "action": "post",
        "paths": ["src/main.rs", "src/a_b.rs"],
        "message": "Keep [this](https://example.com) literal.\nDo not render ![an image](url)."
    }));
    let rendered = markdown(&item).unwrap();
    assert!(rendered.contains("**Action:** post"));
    assert!(rendered.contains("- ` src/a_b.rs `"));
    assert!(rendered.contains("\\[this\\]\\(https\\:\\/\\/example\\.com\\)"));
    assert!(rendered.contains("\\!\\[an image\\]"));
    assert!(
        !rendered.contains("```"),
        "a prose request should not become a code block"
    );
}

#[test]
fn command_and_output_are_separate_and_embedded_fences_stay_literal() {
    let mut item =
        item(json!({"command": "printf '```example```'", "commandActions": [{"type": "unknown"}]}));
    item.tool_output = "example output".into();
    item.is_error = true;
    let rendered = markdown(&item).unwrap();
    assert_eq!(rendered.matches("printf '```example```'").count(), 1);
    assert!(rendered.contains("````text\nprintf '```example```'\n````"));
    assert!(rendered.contains("**Command actions**"));
    assert!(rendered.contains("**Output · failed**"));
    assert!(rendered.contains("example output"));
    assert!(
        rendered.find("example output").unwrap() < rendered.find("**Command actions**").unwrap()
    );
}

#[test]
fn expanded_command_shows_script_and_keeps_raw_arguments() {
    let invocation = "/run/current-system/sw/bin/sh -lc 'cargo test --offline'";
    let item = item(json!({"command": invocation}));
    let rendered = markdown(&item).unwrap();
    assert!(rendered.contains("```text\ncargo test --offline\n```"));
    assert!(!rendered.contains("/run/current-system"));
    assert_eq!(
        item.tool_details.as_ref().unwrap().arguments["command"],
        invocation
    );
}

#[test]
fn structured_preview_bounds_large_values_and_marks_omitted_fields() {
    let huge = "متن`".repeat(400_000);
    for arguments in [
        json!({"message": huge}),
        json!({"command": huge}),
        json!({"paths": vec!["src/path.rs"; 1000]}),
        Value::Object(
            (0..1000)
                .map(|i| (format!("field_{i}"), json!(i)))
                .collect(),
        ),
    ] {
        let rendered = markdown(&item(arguments)).unwrap();
        assert!(rendered.len() < 24 * 1024);
        assert!(rendered.contains("Preview truncated"));
    }
    let mut item = item(json!({"action": "read"}));
    item.tool_output = "\n".repeat(100_000);
    let rendered = markdown(&item).unwrap();
    assert!(rendered.lines().count() < 64);
    assert!(rendered.contains("Preview truncated"));
    item.tool_output = huge;
    let rendered = markdown(&item).unwrap();
    assert!(rendered.len() < 24 * 1024);
    assert!(rendered.contains("Preview truncated"));
}
