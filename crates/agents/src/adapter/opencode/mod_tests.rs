use super::*;

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
