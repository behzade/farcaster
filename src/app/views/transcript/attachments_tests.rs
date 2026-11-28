use super::*;
use crate::conversation::ConversationState;
use serde_json::json;

#[test]
fn image_captions_use_context_without_inspecting_scripts() {
    for (args, metadata, expected) in [
        (
            json!({"title":"  Inspect appearance settings  ","code":"irrelevant"}),
            json!({"title":"js"}),
            "Inspect appearance settings",
        ),
        (
            json!({"title":"  "}),
            json!({"title":"App screenshot"}),
            "App screenshot",
        ),
        (
            json!({"path":"/tmp/composer.png"}),
            json!({"title":"js"}),
            "composer.png",
        ),
        (
            json!({}),
            json!({"targets":["/tmp/settings.png"]}),
            "settings.png",
        ),
        (
            json!({"code":"await capture('invented title')"}),
            json!({"title":"js"}),
            "js",
        ),
    ] {
        let mut state = ConversationState::default();
        state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"capture", "toolName":"js", "args":args, "toolMetadata":metadata}));
        assert_eq!(image_caption(&state.items[0]), expected);
    }
}
