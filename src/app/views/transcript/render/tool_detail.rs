use serde_json::Value;

use crate::conversation::TranscriptItem;

use super::ToolPreview;

const MAX_FIELDS: usize = 32;
const MAX_CHARS: usize = 4096;

// Structured arguments are the source of truth; never parse the adapter's
// human-readable dump to guess which parts are prose or command output.
pub(super) fn markdown(item: &TranscriptItem) -> Option<String> {
    let details = item.tool_details.as_ref()?;
    let arguments = details
        .arguments
        .as_object()
        .filter(|args| !args.is_empty())?;
    let mut body = String::new();
    let mut budget = PreviewBudget {
        chars: MAX_CHARS,
        lines: 40,
        truncated: arguments.len() > MAX_FIELDS,
    };
    let command = details.display_command();
    render_fields(
        &mut body,
        arguments
            .iter()
            .take(MAX_FIELDS)
            .filter(|(_, value)| !nested(value)),
        &mut budget,
        command.as_deref(),
    );
    if !item.tool_output.is_empty() {
        heading(
            &mut body,
            if item.is_error {
                "Output · failed"
            } else {
                "Output"
            },
        );
        body.push_str(&code_block(&budget.take(&item.tool_output)));
    } else if let Some(result) = &details.result {
        let mut preview = ToolPreview::default();
        let _ = serde_json::to_writer_pretty(&mut preview, result);
        heading(&mut body, "Result");
        body.push_str(&code_block(&budget.take(&preview.finish())));
    }
    render_fields(
        &mut body,
        arguments
            .iter()
            .take(MAX_FIELDS)
            .filter(|(_, value)| nested(value)),
        &mut budget,
        command.as_deref(),
    );
    if let Some(review) = &item.tool_review {
        heading(&mut body, "Approval review");
        body.push_str(review.state.label());
        if let Some(detail) = &review.detail {
            body.push_str("\n\n");
            body.push_str(&literal(&budget.take(detail)));
        }
    }
    if budget.truncated {
        body.push_str("\n\nPreview truncated. Copy the tool activity for full details.");
    }
    Some(body)
}

// Complex input metadata follows the result so it cannot bury the output.
fn nested(value: &Value) -> bool {
    value.is_object()
        || value.as_array().is_some_and(|values| {
            values
                .iter()
                .take(MAX_FIELDS)
                .any(|value| !value.is_string())
        })
}

fn render_fields<'a>(
    body: &mut String,
    fields: impl Iterator<Item = (&'a String, &'a Value)>,
    budget: &mut PreviewBudget,
    command: Option<&str>,
) {
    for (key, value) in fields {
        if budget.exhausted() {
            budget.truncated = true;
            break;
        }
        let label = budget.take(key);
        let code = matches!(
            key.as_str(),
            "command"
                | "cmd"
                | "script"
                | "code"
                | "content"
                | "diff"
                | "patch"
                | "old_string"
                | "new_string"
        );
        let path = matches!(
            key.as_str(),
            "path" | "paths" | "file_path" | "file_paths" | "cwd"
        );
        let compact = match value {
            Value::String(text) if !code && !path && text.len() <= 64 && !text.contains('\n') => {
                Some(text.clone())
            }
            Value::Bool(_) | Value::Number(_) | Value::Null => Some(value.to_string()),
            _ => None,
        };
        if let Some(text) = compact {
            body.push_str(&format!(
                "**{}:** {}\n\n",
                literal(&display_label(&label)),
                literal(&budget.take(&text)),
            ));
            continue;
        }
        heading(body, &label);
        if let Some(values) = value.as_array().filter(|values| {
            !values.is_empty() && values.iter().take(MAX_FIELDS).all(Value::is_string)
        }) {
            for (index, value) in values.iter().take(MAX_FIELDS).enumerate() {
                let text = budget.take(value.as_str().unwrap());
                body.push_str("- ");
                body.push_str(&if path {
                    inline_code(&text)
                } else {
                    literal(&text)
                });
                body.push('\n');
                if budget.exhausted() {
                    budget.truncated |= index + 1 < values.len();
                    break;
                }
            }
            budget.truncated |= values.len() > MAX_FIELDS;
        } else {
            let mut preview = ToolPreview::default();
            if let Some(text) = value.as_str() {
                preview.push_str(if key == "command" {
                    command.unwrap_or(text)
                } else {
                    text
                });
            } else {
                let _ = serde_json::to_writer_pretty(&mut preview, value);
            }
            let text = budget.take(&preview.finish());
            body.push_str(&if code || value.is_object() || value.is_array() {
                code_block(&text)
            } else if path {
                inline_code(&text)
            } else {
                literal(&text)
            });
        }
        body.push_str("\n\n");
    }
}

fn heading(body: &mut String, label: &str) {
    body.push_str("\n\n**");
    body.push_str(&literal(&display_label(label)));
    body.push_str("**\n\n");
}

fn display_label(label: &str) -> String {
    let mut words = String::new();
    let mut previous = ' ';
    for ch in label.chars() {
        if ch.is_uppercase() && (previous.is_lowercase() || previous.is_numeric()) {
            words.push(' ');
            words.extend(ch.to_lowercase());
        } else {
            words.push(if ch == '_' { ' ' } else { ch });
        }
        previous = ch;
    }
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

struct PreviewBudget {
    chars: usize,
    lines: usize,
    truncated: bool,
}

impl PreviewBudget {
    fn exhausted(&self) -> bool {
        self.chars == 0 || self.lines == 0
    }

    fn take(&mut self, text: &str) -> String {
        let mut preview = String::new();
        for ch in text.chars() {
            if self.exhausted() {
                self.truncated = true;
                break;
            }
            self.chars -= 1;
            self.lines -= usize::from(ch == '\n');
            preview.push(ch);
        }
        preview
    }
}

// Argument strings are literal data, including Markdown-like instructions.
fn literal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        if ch == '\n' {
            escaped.push_str("  ");
        }
        escaped.push(ch);
    }
    escaped
}

fn fence(text: &str, minimum: usize) -> String {
    "`".repeat(
        text.split(|ch| ch != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(minimum),
    )
}

fn inline_code(text: &str) -> String {
    let fence = fence(text, 1);
    format!("{fence} {text} {fence}")
}

fn code_block(text: &str) -> String {
    let fence = fence(text, 3);
    format!("{fence}text\n{text}\n{fence}\n")
}

#[cfg(test)]
#[path = "tool_detail_tests.rs"]
mod tests;
