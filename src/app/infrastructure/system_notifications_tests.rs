use super::*;

fn notification(tag: &str, body: &str) -> SystemNotification {
    SystemNotification {
        tag: tag.to_owned().into(),
        title: "Farcaster".into(),
        body: body.to_owned().into(),
        actions: Vec::new(),
    }
}

#[test]
fn authorization_keeps_the_first_notice_and_coalesces_later_notices_by_tag() {
    let mut state = NotificationAuthorization::default();
    assert!(state.enqueue(notification("first", "opens permission prompt")));
    assert!(!state.enqueue(notification("second", "old")));
    assert!(!state.enqueue(notification("second", "latest")));
    let delivered = state.resolve(true);
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0].body.as_ref(), "opens permission prompt");
    assert_eq!(delivered[1].body.as_ref(), "latest");
    assert!(state.resolve(true).is_empty());
}

#[test]
fn denial_drops_pending_notices_and_allows_a_later_permission_check() {
    let mut state = NotificationAuthorization::default();
    assert!(state.enqueue(notification("first", "denied")));
    assert!(!state.enqueue(notification("second", "also denied")));
    assert!(state.resolve(false).is_empty());
    assert!(state.enqueue(notification("third", "new permission check")));
    let delivered = state.resolve(true);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].tag.as_ref(), "third");
}
