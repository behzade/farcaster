use std::sync::Arc;

use super::*;
use serde_json::json;

#[test]
fn retrying_actor_blocks_destructive_family_commands() {
    let mut snapshot = RuntimeSnapshot::default();
    assert!(!session_actor_has_active_work(&snapshot, false));
    Arc::make_mut(&mut snapshot.conversation).reduce(&json!({
        "type": "auto_retry_start",
        "attempt": 1,
    }));

    assert!(!snapshot.conversation.running);
    assert!(snapshot.conversation.retrying);
    assert!(session_actor_has_active_work(&snapshot, false));
}
