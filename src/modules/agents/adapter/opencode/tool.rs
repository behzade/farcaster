use serde_json::Value;

use crate::agents::CommonTool;

pub(super) fn normalize_opencode_tool(name: &str, arguments: &Value) -> (String, Value) {
    let normalized_name = name.trim().to_ascii_lowercase();
    let common = CommonTool::from_name(&normalized_name).or(match normalized_name.as_str() {
        "read_file" => Some(CommonTool::Read),
        "write_file" => Some(CommonTool::Write),
        "edit_file" | "apply_patch" | "patch" => Some(CommonTool::Edit),
        "shell" | "command" | "terminal" => Some(CommonTool::Bash),
        _ => None,
    });
    let Some(common) = common else {
        return (name.to_owned(), arguments.clone());
    };
    let mut normalized = arguments.as_object().cloned().unwrap_or_default();
    rename_argument(&mut normalized, "path", &["file_path", "filePath"]);
    if common == CommonTool::Edit {
        rename_argument(&mut normalized, "oldText", &["old_string", "oldString"]);
        rename_argument(&mut normalized, "newText", &["new_string", "newString"]);
    } else if common == CommonTool::Bash {
        rename_argument(&mut normalized, "command", &["cmd"]);
    }
    (common.name().into(), Value::Object(normalized))
}

fn rename_argument(
    arguments: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) {
    let value = aliases.iter().find_map(|alias| arguments.remove(*alias));
    if !arguments.contains_key(canonical)
        && let Some(value) = value
    {
        arguments.insert(canonical.into(), value);
    }
    for alias in aliases {
        arguments.remove(*alias);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn common_tools_use_shared_names_and_arguments() {
        for (source, canonical) in [
            ("read_file", "read"),
            ("write_file", "write"),
            ("apply_patch", "edit"),
            ("shell", "bash"),
        ] {
            assert_eq!(normalize_opencode_tool(source, &json!({})).0, canonical);
        }
        assert_eq!(
            normalize_opencode_tool(
                "edit",
                &json!({"filePath": "src/main.rs", "oldString": "old", "newString": "new"}),
            )
            .1,
            json!({"path": "src/main.rs", "oldText": "old", "newText": "new"})
        );
    }
}
