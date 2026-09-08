use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, OnceLock},
};

use serde_json::Value;

// A separate catalog app-server reports live children as notLoaded. Keep the
// lifecycle observed by their owning connection until that connection closes.
fn states() -> MutexGuard<'static, HashMap<String, (String, bool)>> {
    static STATES: OnceLock<Mutex<HashMap<String, (String, bool)>>> = OnceLock::new();
    STATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(super) fn observe(parent: &str, item: &Value) {
    let Some(child) = item.get("agentThreadId").and_then(Value::as_str) else {
        return;
    };
    let running = match item.get("kind").and_then(Value::as_str) {
        Some("started" | "interacted") => true,
        Some("interrupted" | "completed") => false,
        _ => return,
    };
    states().insert(child.to_owned(), (parent.to_owned(), running));
}

pub(super) fn is_running(child: &str) -> Option<bool> {
    states().get(child).map(|(_, running)| *running)
}

pub(super) fn forget_parent(parent: &str) {
    states().retain(|_, (owner, _)| owner != parent);
}
