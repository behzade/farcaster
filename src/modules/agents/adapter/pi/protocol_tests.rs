use super::*;
use crate::agents::extensions::PromptImage;

#[test]
fn encodes_pi_requests_only_at_the_adapter_boundary() {
    assert_eq!(
        encode_request(SessionCommand::ApplySteering),
        json!({"type":"abort"})
    );
    assert_eq!(
        encode_request(SessionCommand::ConfigureSteering),
        json!({"type":"set_steering_mode","mode":"all"})
    );
    assert_eq!(
        encode_request(SessionCommand::Prompt {
            mode: PromptMode::FollowUp,
            message: "later".into(),
            images: vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())],
        }),
        json!({
            "type":"follow_up",
            "message":"later",
            "images":[{"type":"image","data":"aGVsbG8=","mimeType":"image/png"}],
        })
    );
    assert_eq!(
        encode_request(SessionCommand::Compact { instructions: None }),
        json!({"type":"compact"})
    );
}
