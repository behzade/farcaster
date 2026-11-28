use std::borrow::Cow;

use serde_json::Value;

pub fn normalize_tool_output(blocks: &[Value]) -> Cow<'_, [Value]> {
    fn expand(blocks: &[Value], depth: usize) -> Cow<'_, [Value]> {
        let mut output: Option<Vec<Value>> = None;
        for (index, block) in blocks.iter().enumerate() {
            let wrapped = (depth < 4 && block["type"] == "text")
                .then(|| block["text"].as_str())
                .flatten()
                .and_then(|text| serde_json::from_str::<Value>(text).ok());
            let content = wrapped
                .as_ref()
                .and_then(Value::as_object)
                .filter(|fields| {
                    fields.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "content" | "isError" | "structuredContent" | "_meta"
                        )
                    })
                })
                .and_then(|fields| fields.get("content"))
                .and_then(Value::as_array)
                .filter(|content| {
                    !content.is_empty()
                        && content.iter().all(|part| {
                            match part.get("type").and_then(Value::as_str) {
                                Some("text") => part.get("text").is_some_and(Value::is_string),
                                Some("image" | "audio") => {
                                    part.get("data").is_some_and(Value::is_string)
                                        && part.get("mimeType").is_some_and(Value::is_string)
                                }
                                Some("resource_link") => {
                                    part.get("uri").is_some_and(Value::is_string)
                                }
                                Some("resource") => {
                                    part.get("resource").is_some_and(Value::is_object)
                                }
                                _ => false,
                            }
                        })
                });
            if let Some(content) = content {
                output
                    .get_or_insert_with(|| blocks[..index].to_vec())
                    .extend(expand(content, depth + 1).into_owned());
            } else if let Some(output) = &mut output {
                output.push(block.clone());
            }
        }
        output.map_or(Cow::Borrowed(blocks), Cow::Owned)
    }
    expand(blocks, 0)
}

#[cfg(test)]
#[path = "tool_output_tests.rs"]
mod tests;
