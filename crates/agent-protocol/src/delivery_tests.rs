use super::*;
use crate::SessionActivity;
use serde_json::json;

#[test]
fn delivery_boundary_preserves_ids_images_modes_and_native_metadata() {
    let value = json!({"type":"prompt_delivery", "submissionId":"exact-id", "status":"delivered",
        "message":{"role":"user", "content":[{"type":"text", "text":"same"},
            {"type":"image", "data":"AQID", "mimeType":"image/png"}],
            "queued":true, "deliveryTracked":true, "promptMode":"follow_up",
            "farcasterDisplayMessage":"shown input", "timestamp":123}});
    let activity = SessionActivity::from(value.clone());
    let delivery = activity.prompt_delivery().expect("validated delivery");
    assert_eq!(delivery.submission_id, "exact-id");
    assert_eq!(delivery.status, DeliveryStatus::Delivered);
    let message = delivery.message.as_ref().unwrap();
    assert_eq!(message.prompt_mode, Some(PromptMode::FollowUp));
    assert!(message.queued && message.delivery_tracked);
    assert_eq!(SessionActivity::from(delivery.clone()).value(), &value);
}

#[test]
fn malformed_delivery_never_becomes_an_internal_receipt() {
    let valid = json!({"type":"prompt_delivery", "submissionId":"id", "status":"delivered"});
    for (field, value) in [
        ("submissionId", Value::Null),
        ("submissionId", json!("")),
        ("submissionId", json!(12)),
        ("status", Value::Null),
        ("status", json!("future_status")),
        ("message", json!({"role":"assistant"})),
        ("message", json!({"queued":"true"})),
        ("message", json!({"deliveryTracked":"true"})),
        ("message", json!({"promptMode":"queue"})),
    ] {
        let mut event = valid.clone();
        event[field] = value;
        assert!(
            SessionActivity::from(event).prompt_delivery().is_none(),
            "{field}"
        );
    }
    let receipt = SessionActivity::from(valid);
    assert!(receipt.prompt_delivery().unwrap().message.is_none());
}
