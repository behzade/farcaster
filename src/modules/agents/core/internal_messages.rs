//! Model-visible application messages that Farcaster omits from its transcript UI.

use serde_json::Value;

const SANDBOX_GRANT_PREFIX: &str = "<farcaster-internal kind=\"sandbox-grant-activated\">";

pub(crate) fn sandbox_grant_continuation() -> String {
    format!(
        "{SANDBOX_GRANT_PREFIX}\nSandbox access requested in the previous turn is now active. Continue the interrupted task and retry the blocked operation. Account for any actions that completed before interruption.\n</farcaster-internal>"
    )
}

pub(crate) fn is_hidden_text(text: &str) -> bool {
    text.starts_with(SANDBOX_GRANT_PREFIX)
}

pub(crate) fn is_hidden_user_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
        && first_text(message).is_some_and(is_hidden_text)
}

fn first_text(message: &Value) -> Option<&str> {
    let content = message.get("content")?;
    content.as_str().or_else(|| {
        content
            .as_array()?
            .iter()
            .find_map(|block| block.get("text").and_then(Value::as_str))
    })
}
