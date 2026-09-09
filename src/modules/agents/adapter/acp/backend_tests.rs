use super::*;
use crate::modules::agents::adapter::{self, antigravity::PROFILE as ANTIGRAVITY};
use std::path::Path;

#[test]
fn external_agents_have_registered_workers_and_valid_session_identities() {
    let (workers, _) = adapter::worker_factories(crate::agents::AgentLaunchConfig::default());
    for profile in [&ANTIGRAVITY] {
        assert!(workers.contains_key(profile.backend));
        let descriptor = adapter::known_backend_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id.as_str() == profile.backend)
            .unwrap();
        assert_eq!(descriptor.name, profile.name);
        assert_eq!(
            descriptor.capabilities.sessions.fork,
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            descriptor.capabilities.sessions.delete,
            CapabilitySupport::Unsupported
        );
        let path = adapter::main_session::external_session_path(
            Path::new("/sessions"),
            profile.backend,
            "one",
        );
        assert_eq!(
            adapter::external_session_identity(&path),
            Some((profile.backend, "one".into()))
        );
        adapter::session_storage::validate_session_locator(profile.backend, &path).unwrap();
    }
}
