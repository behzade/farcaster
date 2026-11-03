use super::*;

#[test]
fn family_locator_keeps_sha256_directory_names() {
    let root = Path::new("/locators");
    for (project, expected) in [
        (
            "/farcaster-sha2-compat/project",
            "bff63061305baee2040bccea061f55d46e1d26bed2f1bab8a4b2cd706e0b58c9",
        ),
        (
            "/farcaster-sha2-compat/project-10",
            "00cbd62fe5053afd256eb7082afa98fed5c2b5a4c2acd33b8372a0cc732ed1a7",
        ),
    ] {
        assert_eq!(
            family_locator_root(root, Path::new(project)),
            root.join(expected)
        );
    }
}
