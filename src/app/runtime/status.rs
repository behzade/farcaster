use super::*;

pub(super) fn semantic_status(snapshot: &RuntimeSnapshot) -> &'static str {
    if snapshot.history_preview {
        return if snapshot.selected_session.is_none() {
            "Draft"
        } else {
            "Done"
        };
    }
    if snapshot.conversation.running {
        "Working"
    } else if snapshot.conversation.ended_in_error() {
        "Failed"
    } else {
        "Done"
    }
}

pub(super) fn tool_starts_worker(kind: &SessionActivityKind, event: &Value) -> bool {
    if kind != &SessionActivityKind::ToolStarted {
        return false;
    }
    let Some(name) = event.get("toolName").and_then(Value::as_str) else {
        return false;
    };
    let normalized = name
        .trim()
        .rsplit(['.', ':', '/'])
        .next()
        .unwrap_or_default()
        .rsplit("__")
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if normalized == "worker_send" {
        // A top-level caller creates a child when the requested name is new.
        // Refreshing after an existing-child or child-to-parent send is harmless.
        return true;
    }
    matches!(
        normalized.as_str(),
        "spawn_agent" | "spawnagent" | "worker_start"
    )
}

pub(super) fn failure_details(error: &str) -> String {
    let cleaned = error
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect::<String>();
    truncate_chars(cleaned.trim(), MAX_FAILURE_DETAILS_CHARS)
}

pub(super) fn failure_summary(details: &str) -> String {
    let preferred = details.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("Error:")
            .map(str::trim)
            .filter(|line| !line.is_empty())
    });
    let fallback = details.lines().rev().find_map(|line| {
        let line = line.trim();
        (!line.is_empty() && !line.starts_with("Warning:") && !line.starts_with("Hint:"))
            .then_some(line)
    });
    truncate_chars(
        preferred
            .or(fallback)
            .unwrap_or("Pi exited without an error message."),
        MAX_FAILURE_SUMMARY_CHARS,
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut truncated = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        truncated.push('…');
    }
    truncated
}

pub(super) fn run_status(conversation: &ConversationState) -> &'static str {
    if conversation.compacting {
        "Compacting"
    } else if conversation.retrying {
        "Retrying"
    } else if conversation.running {
        "Working"
    } else if conversation.settled {
        "Ready"
    } else {
        "Idle"
    }
}

pub(super) fn notification_target(snapshot: &RuntimeSnapshot) -> Option<(PathBuf, PathBuf)> {
    snapshot
        .live_session
        .clone()
        .or_else(|| snapshot.selected_session.clone())
        .map(|path| (path, snapshot.project.clone()))
}

pub(super) fn session_badge_status(conversation: &ConversationState) -> &'static str {
    if conversation.compacting {
        "Compacting"
    } else if conversation.retrying {
        "Retrying"
    } else if conversation.running {
        "Working"
    } else if conversation.ended_in_error() {
        "Failed"
    } else {
        "Done"
    }
}
