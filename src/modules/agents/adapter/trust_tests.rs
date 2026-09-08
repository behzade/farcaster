use super::*;

#[test]
fn other_backends_do_not_read_pi_resources() {
    for backend in ["codex-cli", "cursor-cli", "opencode2"] {
        let nonexistent = Path::new("/nonexistent/farcaster-trust-test");
        assert_eq!(project_trust(backend, nonexistent), Ok(StartupTrust::Ready));
        assert_eq!(saved_project_trust(backend, nonexistent), Ok(None));
        assert!(project_trust_description(backend).is_none());
    }
}
