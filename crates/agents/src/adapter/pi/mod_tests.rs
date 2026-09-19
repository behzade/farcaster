use super::*;

#[test]
fn descriptor_does_not_collapse_independent_features() {
    let descriptor = descriptor();
    assert_eq!(
        descriptor.capabilities.turns.steer,
        CapabilitySupport::Available
    );
    assert_eq!(
        descriptor.capabilities.turns.follow_up,
        CapabilitySupport::Available
    );
    assert_eq!(
        descriptor.capabilities.configuration.modes,
        CapabilitySupport::Unsupported
    );
    assert_eq!(
        descriptor.capabilities.observation.child_agents,
        CapabilitySupport::Unsupported
    );
}
