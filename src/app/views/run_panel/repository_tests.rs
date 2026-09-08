use super::*;

#[test]
fn auto_does_not_select_jj_before_discovery() {
    assert_eq!(
        selected_backend(None, BackendPreference::Auto, true, true),
        None
    );
    assert_eq!(
        selected_backend(None, BackendPreference::Jujutsu, true, false),
        None
    );
    assert_eq!(
        selected_backend(
            Some(RepositoryKind::Git),
            BackendPreference::Auto,
            true,
            true,
        ),
        Some(RepositoryKind::Git)
    );
}
