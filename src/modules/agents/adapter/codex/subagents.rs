use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, OnceLock},
};

use serde_json::Value;

#[cfg(test)]
#[path = "subagents_tests.rs"]
mod tests;

// A separate catalog app-server reports live children as notLoaded. Keep the
// lifecycle observed by their owning connection until that connection closes.
fn states() -> MutexGuard<'static, HashMap<String, (String, bool)>> {
    static STATES: OnceLock<Mutex<HashMap<String, (String, bool)>>> = OnceLock::new();
    STATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(super) fn observe(parent: &str, item: &Value) -> Option<bool> {
    let child = item.get("agentThreadId").and_then(Value::as_str)?;
    let running = match item.get("kind").and_then(Value::as_str) {
        Some("started") => true,
        Some("interrupted" | "completed") => false,
        // Message delivery does not start a turn, even when the child is idle.
        _ => return None,
    };
    states().insert(child.to_owned(), (parent.to_owned(), running));
    Some(running)
}

pub(super) fn is_running(child: &str) -> Option<bool> {
    states().get(child).map(|(_, running)| *running)
}

pub(super) fn observe_thread(parent: &str, child: &str, thread: &Value) -> Option<bool> {
    let running = match thread.pointer("/status/type").and_then(Value::as_str) {
        Some("active") => true,
        Some("idle" | "systemError") => false,
        // An unloaded thread can still have a turn running in another process.
        _ => match thread
            .get("turns")?
            .as_array()?
            .last()?
            .get("status")?
            .as_str()?
        {
            "inProgress" => true,
            "completed" | "interrupted" | "failed" => false,
            _ => return None,
        },
    };
    states().insert(child.to_owned(), (parent.to_owned(), running));
    Some(running)
}

pub(super) fn forget_parent(parent: &str) {
    states().retain(|_, (owner, _)| owner != parent);
}
