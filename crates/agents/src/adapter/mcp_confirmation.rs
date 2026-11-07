use serde_json::{Value, json};

pub(super) fn prompt(
    server: &str,
    message: &str,
    mode: &str,
    schema: &Value,
    metadata: &Value,
) -> Result<String, &'static str> {
    let Some(schema) = schema.as_object() else {
        return Err("MCP confirmation is missing its form schema");
    };
    // A confirmation has no values to collect. Never treat an unsupported form
    // or a URL authorization flow as permission to submit an empty object.
    if mode != "form"
        || server.trim().is_empty()
        || message.trim().is_empty()
        || schema.get("type").and_then(Value::as_str) != Some("object")
        || !schema.iter().all(|(key, value)| match key.as_str() {
            "properties" => value
                .as_object()
                .is_some_and(|properties| properties.is_empty()),
            "required" => value.as_array().is_some_and(|required| required.is_empty()),
            "additionalProperties" => value.is_boolean(),
            "type" | "title" | "description" | "$schema" => true,
            _ => false,
        })
    {
        return Err("Only MCP confirmation forms without input fields are supported");
    }
    let mut prompt = format!("{message}\nRequested by: {server}");
    for (key, label) in [
        ("subtitle", "Note"),
        ("tool_name", "Tool"),
        ("riskLevel", "Risk"),
    ] {
        if let Some(value) = metadata[key].as_str() {
            prompt.push_str(&format!("\n{label}: {value}"));
        }
    }
    if let Some(arguments) = metadata.get("tool_params") {
        prompt.push_str(&format!("\nRequested action:\n{arguments:#}"));
    }
    Ok(prompt)
}

pub(super) fn reply(accepted: bool, cancel: bool) -> Value {
    json!({
        "action": if cancel { "cancel" } else if accepted { "accept" } else { "decline" },
        "content": if accepted && !cancel { json!({}) } else { Value::Null },
    })
}

#[cfg(test)]
#[path = "mcp_confirmation_tests.rs"]
mod tests;
