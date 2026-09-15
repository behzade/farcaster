use serde_json::{Value, json};

pub(super) fn catalog(response: Value) -> Result<Vec<Value>, String> {
    let rows = response
        .as_array()
        .or_else(|| response.get("data").and_then(Value::as_array))
        .ok_or("OpenCode returned an invalid command catalog")?;
    let mut commands = vec![json!({
        "name": "compact", "description": "Compact this session's context", "source": "extension"
    })];
    for row in rows {
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .ok_or("OpenCode command has no name")?;
        if name != "compact" {
            commands.push(json!({
                "name": name, "description": row.get("description").and_then(Value::as_str), "source": "prompt"
            }));
        }
    }
    Ok(commands)
}

pub(super) fn invocation<'a>(
    message: &'a str,
    commands: &std::collections::HashSet<String>,
) -> Option<(&'a str, &'a str)> {
    let input = message.trim_start().strip_prefix('/')?;
    let (name, text) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    (name == "compact" || commands.contains(name)).then_some((name, text.trim_start()))
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
