use super::*;

fn index_of_icon(name: &str) -> usize {
    index_of(name).expect("bundled icon")
}

#[test]
fn brand_colors_come_from_the_bundled_svgs() {
    assert_eq!(
        native_color(index_of_icon("rust")),
        parse_hex("#ff7043").expect("hex")
    );
    assert_eq!(
        native_color(index_of_icon("python")),
        parse_hex("#0288d1").expect("hex")
    );
    assert!(ICON_NAMES.iter().all(|name| index_of(name).is_some()));
}

#[test]
fn recoloring_replaces_brand_fills_and_keeps_structure() {
    let rust = index_of_icon("rust");
    let tinted = recolor(svg(rust), parse_hex("#123456").expect("hex"));
    assert!(tinted.contains("fill=\"#123456\""));
    assert!(!tinted.contains("#ff7043"));
    assert!(tinted.starts_with("<svg"));
    assert!(tinted.ends_with("</svg>"));

    let kotlin = index_of_icon("kotlin");
    let tinted = recolor(svg(kotlin), parse_hex("#123456").expect("hex"));
    assert!(tinted.contains("fill=\"url(#a)\""));
}

#[test]
fn icon_tokens_use_the_bundled_file_names() {
    assert_eq!(
        token_name(index_of_icon("typescript")).as_deref(),
        Some("icon-typescript")
    );
    assert_eq!(token_name(ICON_NAMES.len()), None);
}

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
