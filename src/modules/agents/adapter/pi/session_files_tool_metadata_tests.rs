use serde_json::json;

use super::*;

#[test]
fn loaded_pi_history_annotates_tool_calls_without_changing_arguments() {
    let arguments = json!({"path": "README.md", "offset": 4});
    let messages = project_display_history(&[json!({
        "type": "message",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": "read-1",
                "name": "read",
                "arguments": arguments.clone()
            }]
        }
    })]);
    let block = &messages[0]["content"][0];
    assert_eq!(block["arguments"], arguments);
    assert_eq!(block["toolMetadata"]["category"], "read");
    assert_eq!(block["toolMetadata"]["targets"], json!(["README.md"]));
    assert_eq!(block["toolMetadata"]["native"], arguments);
}
