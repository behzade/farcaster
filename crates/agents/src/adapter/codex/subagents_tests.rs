use super::*;
use serde_json::json;

#[test]
fn child_status_uses_the_latest_turn_when_unloaded() {
    let parent = "thread-read-parent";
    let child = "thread-read-child";
    for (status, turn, running) in [
        ("notLoaded", "completed", false),
        ("notLoaded", "inProgress", true),
        ("active", "completed", true),
        ("idle", "completed", false),
        ("notLoaded", "interrupted", false),
        ("notLoaded", "failed", false),
    ] {
        assert_eq!(
            observe_thread(
                parent,
                child,
                &json!({
                    "status": {"type": status},
                    "turns": [{"status": "inProgress"}, {"status": turn}]
                })
            ),
            Some(running)
        );
        assert_eq!(is_running(child), Some(running));
    }
    assert_eq!(
        observe_thread(parent, child, &json!({"status": {"type": "notLoaded"}})),
        None
    );
    assert_eq!(is_running(child), Some(false));
    forget_parent(parent);
}
