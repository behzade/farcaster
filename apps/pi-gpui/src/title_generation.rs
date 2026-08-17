//! Semantic session-title generation through a short-lived Pi process.

use std::{
    io::Write as _,
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use crate::{
    conversation::{ConversationState, TranscriptKind},
    rpc_process::ProcessCommand,
};

const DEFAULT_TITLE_MODEL: &str = "openai/gpt-5.6-luna";
const MAX_CONTEXT_CHARS: usize = 4_000;
const MAX_TITLE_CHARS: usize = 80;
const MAX_TITLE_WORDS: usize = 12;
const TITLE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn configured_model() -> String {
    std::env::var("PI_GUI_TITLE_MODEL")
        .ok()
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| DEFAULT_TITLE_MODEL.to_owned())
}

pub(crate) fn eligible_session(
    session_name: Option<&str>,
    session_file: Option<&str>,
    attempted: Option<&Path>,
) -> Option<std::path::PathBuf> {
    if session_name.is_some_and(|name| !name.trim().is_empty()) {
        return None;
    }
    let path = session_file.map(std::path::PathBuf::from)?;
    (attempted != Some(path.as_path())).then_some(path)
}

pub(crate) fn accepts_result(
    session_name: Option<&str>,
    session_file: Option<&str>,
    generated_path: &Path,
) -> bool {
    session_name.is_none_or(|name| name.trim().is_empty())
        && session_file
            .map(std::path::PathBuf::from)
            .is_some_and(|path| {
                crate::sessions::normalize_session_path(&path)
                    == crate::sessions::normalize_session_path(generated_path)
            })
}

pub(crate) fn title_context(conversation: &ConversationState) -> Option<String> {
    let user = conversation
        .items
        .iter()
        .find(|item| item.kind == TranscriptKind::User && !item.text.trim().is_empty())?;
    let assistant = conversation
        .items
        .iter()
        .skip_while(|item| !std::sync::Arc::ptr_eq(item, user))
        .find(|item| item.kind == TranscriptKind::Assistant && !item.text.trim().is_empty())?;
    let context = format!(
        "User request:\n{}\n\nAssistant response:\n{}",
        user.text.trim(),
        assistant.text.trim()
    );
    Some(truncate_chars(&context, MAX_CONTEXT_CHARS))
}

pub(crate) fn generate(
    command: &ProcessCommand,
    project: &Path,
    model: &str,
    context: &str,
) -> Result<String, String> {
    let mut process = command.command(project);
    process
        .args([
            "--print",
            "--no-session",
            "--no-context-files",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-tools",
            "--thinking",
            "off",
            "--model",
            model,
            "--system-prompt",
            "Create a concise semantic title for this coding session. Return only the title, without quotes, markdown, or punctuation at the end. Use at most 12 words.",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process
        .spawn()
        .map_err(|error| format!("start title model {model}: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "title model stdin was not piped".to_owned())?
        .write_all(context.as_bytes())
        .map_err(|error| format!("send title context: {error}"))?;
    let deadline = Instant::now() + TITLE_TIMEOUT;
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("check title model: {error}"))?
        {
            Some(_) => break,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("title model timed out after 30 seconds".to_owned());
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("read title model output: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("title model exited with {}", output.status)
        } else {
            format!("title model failed: {detail}")
        });
    }
    normalize(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "title model returned an empty title".to_owned())
}

pub(crate) fn normalize(value: &str) -> Option<String> {
    let first_line = value.lines().find(|line| !line.trim().is_empty())?.trim();
    let unquoted = first_line
        .strip_prefix('"')
        .and_then(|line| line.strip_suffix('"'))
        .or_else(|| {
            first_line
                .strip_prefix('`')
                .and_then(|line| line.strip_suffix('`'))
        })
        .unwrap_or(first_line)
        .trim_matches(['"', '`'])
        .trim()
        .trim_end_matches(['.', ':', ';']);
    let words = unquoted
        .split_whitespace()
        .take(MAX_TITLE_WORDS)
        .collect::<Vec<_>>()
        .join(" ");
    let title = truncate_chars(&words, MAX_TITLE_CHARS).trim().to_owned();
    (!title.is_empty()).then_some(title)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_model_output_to_a_bounded_single_line_title() {
        assert_eq!(
            normalize("\n\"Implement automatic semantic session titles.\"\nextra"),
            Some("Implement automatic semantic session titles".into())
        );
        assert_eq!(normalize("```"), None);
        assert!(normalize(&"word ".repeat(20)).is_some_and(|title| {
            title.split_whitespace().count() == MAX_TITLE_WORDS
                && title.chars().count() <= MAX_TITLE_CHARS
        }));
    }

    #[test]
    fn existing_or_already_attempted_sessions_are_not_generated_again() {
        let path = Path::new("/session.jsonl");
        assert_eq!(
            eligible_session(Some("Manual title"), Some("/session.jsonl"), None),
            None
        );
        assert_eq!(
            eligible_session(None, Some("/session.jsonl"), Some(path)),
            None
        );
        assert_eq!(
            eligible_session(None, Some("/session.jsonl"), None),
            Some(path.to_path_buf())
        );
        assert!(!accepts_result(
            Some("Manual title"),
            Some("/session.jsonl"),
            path
        ));
        assert!(accepts_result(None, Some("/session.jsonl"), path));
        assert!(!accepts_result(None, Some("/other.jsonl"), path));
    }

    #[cfg(unix)]
    #[test]
    fn generation_runs_the_configured_pi_command_and_normalizes_stdout()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        let temp = tempfile::tempdir()?;
        let script = temp.path().join("fake-pi");
        fs::write(
            &script,
            "#!/bin/sh\ncat >/dev/null\nprintf '\"Generated semantic title.\"\\n'\n",
        )?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        let command = ProcessCommand {
            program: script,
            prefix_args: Vec::new(),
            direnv_program: None,
        };

        assert_eq!(
            generate(&command, temp.path(), "test/lite", "session context")?,
            "Generated semantic title"
        );
        Ok(())
    }

    #[test]
    fn title_context_requires_a_user_and_assistant_exchange() {
        let mut conversation = ConversationState::default();
        conversation.push_local_user("Build title generation".into(), 0);
        assert_eq!(title_context(&conversation), None);
        conversation.replace_history(&[
            serde_json::json!({"role":"user","content":"Build title generation"}),
            serde_json::json!({
                "role":"assistant",
                "content":[{"type":"text","text":"I implemented it."}],
                "stopReason":"stop"
            }),
        ]);
        assert_eq!(
            title_context(&conversation),
            Some(
                "User request:\nBuild title generation\n\nAssistant response:\nI implemented it."
                    .into()
            )
        );
    }
}
