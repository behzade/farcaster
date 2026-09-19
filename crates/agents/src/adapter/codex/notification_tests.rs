use super::*;

#[test]
fn native_aliases_map_to_canonical_methods() {
    for (native, expected) in [
        ("execCommandApproval", CodexMethod::CommandApproval),
        ("applyPatchApproval", CodexMethod::FileChangeApproval),
        (
            "item/reasoning/summaryTextDelta",
            CodexMethod::ReasoningTextDelta,
        ),
    ] {
        assert_eq!(CodexMethod::parse(native), expected);
    }
    let unknown = CodexMethod::parse("future/thing");
    assert_eq!(unknown, CodexMethod::Unknown("future/thing"));
}

#[test]
fn methods_expose_dispatch_tier_and_request_kind() {
    use CodexNotificationTier::{Global, Skills, Telemetry, Thread};

    for (method, tier) in [
        ("skills/changed", Skills),
        ("mcpServer/startupStatus/updated", Telemetry),
        ("account/rateLimits/updated", Telemetry),
        ("warning", Global),
        ("configWarning", Global),
        ("remoteControl/status/changed", Global),
        ("thread/started", Thread),
        ("turn/started", Thread),
        ("future/thing", Thread),
    ] {
        assert_eq!(CodexMethod::parse(method).tier(), tier);
    }
    assert!(CodexMethod::parse("execCommandApproval").is_approval_request());
    assert!(CodexMethod::PermissionsApproval.is_approval_request());
    assert!(!CodexMethod::Unknown("future/thing").is_approval_request());
}
