use super::*;

#[test]
fn recognizes_names_before_extensions_and_ignores_directories() {
    for (path, expected) in [
        ("src/main.rs", "rust"),
        ("src/APP.TSX", "react"),
        ("Cargo.toml", "rust"),
        ("other.toml", "config"),
        ("sub/Dockerfile", "docker"),
        (".gitignore", "git"),
        (".env.local", "config"),
        ("docs/README.md", "markdown"),
        ("locales/fa/pdp.json", "json"),
        ("settings.jsonc", "json"),
        ("events.jsonl", "json"),
        ("src.rs/unknown", "file"),
        ("unknown.xyz", "file"),
        ("", "file"),
    ] {
        assert_eq!(classify(Path::new(path)), expected, "{path}");
    }
}
