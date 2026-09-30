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
    pending.submitted("normal".into(), PromptMode::Normal, "2", &[]);
    let mut event = json!({"type":"message_start", "message":{"role":"user", "content":"2"}});
    assert!(matches!(
        pending.observe(&event),
        Some(SessionEvent::Stderr(_))
    ));
    assert_eq!(pending.0.len(), 5);
    event["type"] = "message_end".into();
    for id in ["normal", "first", "second", "follow-up"] {
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

#[test]
fn malformed_receipts_keep_pending_delivery_and_valid_receipts_keep_native_fields() {
    let mut pending = Deliveries::default();
    pending.submitted("id".into(), PromptMode::Normal, "hello", &[]);
    let mut event = json!({
        "type": "message_end",
        "message": {
            "role": "user",
            "content": [{"type":"text", "text":"hello"}],
            "queued": "invalid",
            "timestamp": 123,
        },
    });
    assert!(pending.observe(&event).is_none());
    assert!(!pending.is_empty());
    event["message"]["queued"] = false.into();
    let Some(SessionEvent::Activity(receipt)) = pending.observe(&event) else {
        panic!("missing valid receipt")
    };
    let message = receipt.prompt_delivery().unwrap().message.as_ref().unwrap();
    assert_eq!(message.metadata["timestamp"], 123);
    assert_eq!(message.content, event["message"]["content"]);
    assert!(message.delivery_tracked);
    assert!(pending.is_empty());
}
