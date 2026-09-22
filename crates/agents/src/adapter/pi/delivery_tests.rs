use super::*;

#[test]
fn native_events_consume_each_occurrence_once_and_keep_image_identity() {
    let mut pending = Deliveries::default();
    let image = PromptImage::new("aGVsbG8=".into(), "image/png".into());
    pending.submitted("image".into(), PromptMode::Steer, "2", &[image]);
    pending.submitted("follow-up".into(), PromptMode::FollowUp, "2", &[]);
    for id in ["first", "second"] {
        pending.submitted(id.into(), PromptMode::Steer, "2", &[]);
    }
    let mut event = json!({"type":"message_start", "message":{"role":"user", "content":"2"}});
    assert!(matches!(
        pending.observe(&event),
        Some(SessionEvent::Stderr(_))
    ));
    assert_eq!(pending.0.len(), 4);
    event["type"] = "message_end".into();
    for id in ["first", "second", "follow-up"] {
        let Some(SessionEvent::Activity(receipt)) = pending.observe(&event) else {
            panic!("missing receipt")
        };
        assert_eq!(receipt.value()["submissionId"], id);
        assert_eq!(receipt.value()["status"], "delivered");
    }
    assert!(pending.observe(&event).is_none());
    event["message"]["content"] = pending.0[0].2.clone();
    let Some(SessionEvent::Activity(receipt)) = pending.observe(&event) else {
        panic!("missing image receipt")
    };
    assert_eq!(receipt.value()["submissionId"], "image");
    assert!(pending.0.is_empty());
}
