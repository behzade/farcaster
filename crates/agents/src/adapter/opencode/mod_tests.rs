use super::*;

#[test]
fn model_override_parses_a_provider_and_model_pair() {
    let selection = parse_model_override("opencode/big-pickle").expect("parsed");
    assert_eq!(selection.provider_id, "opencode");
    assert_eq!(selection.id, "big-pickle");
    assert_eq!(selection.variant, None);
}

#[test]
fn model_override_keeps_a_named_variant_and_drops_the_default_marker() {
    let named = parse_model_override("opencode-go/deepseek-v4-flash#max").expect("parsed");
    assert_eq!(named.provider_id, "opencode-go");
    assert_eq!(named.id, "deepseek-v4-flash");
    assert_eq!(named.variant.as_deref(), Some("max"));

    let default = parse_model_override("opencode/big-pickle#default").expect("parsed");
    assert_eq!(default.variant, None);
}

#[test]
fn model_override_rejects_values_without_a_provider_or_model() {
    assert!(parse_model_override("big-pickle").is_none());
    assert!(parse_model_override("/big-pickle").is_none());
    assert!(parse_model_override("opencode/").is_none());
}

#[test]
fn descriptor_keeps_opencode_specific_features_independent() {
    let capabilities = descriptor().capabilities;
    assert_eq!(capabilities.turns.queue, CapabilitySupport::Available);
    assert_eq!(capabilities.turns.follow_up, CapabilitySupport::Available);
    assert_eq!(
        capabilities.configuration.commands,
        CapabilitySupport::Available
    );
}
