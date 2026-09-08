use serde_json::json;

use super::*;

#[test]
fn edit_patches_match_in_live_events_and_history_without_changing_raw_diff() {
    let details = json!({"diff":"-1 old\n+1 new", "firstChangedLine":1});
    let mut live = json!({"type":"tool_execution_end", "result":{"details":details}});
    let mut history = json!({"role":"toolResult", "details":details});
    annotate_pi_value(&mut live);
    annotate_pi_message(&mut history);
    assert_eq!(live["result"]["details"], history["details"]);
    assert_eq!(history["details"]["diff"], details["diff"]);
    assert_eq!(
        history["details"]["unifiedDiff"],
        "@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
}

#[test]
fn builtin_metadata_matches_across_live_history_and_stream_events() {
    let arguments = json!({"path": "src/lib.rs", "offset": 2});
    let mut live = json!({
        "type": "tool_execution_start",
        "toolCallId": "read-1",
        "toolName": "read",
        "args": arguments.clone()
    });
    let mut history = json!({
        "role": "assistant",
        "content": [{
            "type": "toolCall",
            "id": "read-1",
            "name": "read",
            "arguments": arguments.clone()
        }]
    });
    let mut stream = json!({
        "type": "message_update",
        "assistantMessageEvent": {
            "type": "toolcall_end",
            "toolCall": {
                "type": "toolCall",
                "id": "read-1",
                "name": "read",
                "arguments": arguments.clone()
            }
        }
    });

    annotate_pi_value(&mut live);
    annotate_pi_message(&mut history);
    annotate_pi_value(&mut stream);

    assert_eq!(live["args"], arguments);
    let expected = live["toolMetadata"].clone();
    assert_eq!(expected["category"], "read");
    assert_eq!(expected["targets"], json!(["src/lib.rs"]));
    assert_eq!(history["content"][0]["toolMetadata"], expected);
    assert_eq!(
        stream["assistantMessageEvent"]["toolCall"]["toolMetadata"],
        expected
    );
}

#[test]
fn leaves_custom_tool_intent_unknown_and_keeps_native_metadata() {
    let mut message = json!({
        "role": "assistant",
        "content": [{
            "type": "toolCall",
            "id": "custom-1",
            "name": "mcp_custom_action",
            "arguments": {"path": "do-not-guess"},
            "toolMetadata": {"native": {"provider": "custom"}}
        }]
    });
    annotate_pi_message(&mut message);
    let metadata = &message["content"][0]["toolMetadata"];
    assert_eq!(metadata["category"], "other");
    assert!(metadata["title"].is_null());
    assert!(metadata.get("targets").is_none());
    assert_eq!(metadata["native"], json!({"provider": "custom"}));
}
