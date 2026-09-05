//! Bundled Devicon v2.17.0 icons; monochrome glyphs get dark-theme tints.
//! File, config, and image fallback glyphs are original Farcaster artwork.
use std::{borrow::Cow, path::Path};

use gpui::{AnyElement, IntoElement as _, Styled as _, img, rgb, svg};

use super::theme::THEME;

macro_rules! file_assets {
    ($($name:literal),* $(,)?) => {
        pub(super) const ASSETS: &[(&str, &[u8])] = &[$(
            (concat!("icons/files/", $name, ".svg"),
             include_bytes!(concat!("../../../assets/file-icons/", $name, ".svg"))),
        )*];
    };
}

file_assets!(
    "bash",
    "c",
    "config",
    "cplusplus",
    "csharp",
    "css3",
    "docker",
    "file",
    "git",
    "go",
    "html5",
    "image",
    "java",
    "javascript",
    "json",
    "kotlin",
    "markdown",
    "nixos",
    "python",
    "react",
    "ruby",
    "rust",
    "sass",
    "svelte",
    "swift",
    "typescript",
    "vuejs",
    "yaml",
);

pub(super) fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| Cow::Borrowed(*bytes))
}

pub(crate) fn file_icon(path: &Path) -> AnyElement {
    let name = classify(path);
    let asset = format!("icons/files/{name}.svg");
    let tint = match name {
        "rust" => Some(rgb(0xdea584)),
        "markdown" => Some(rgb(0x8bb8e8)),
        "yaml" => Some(rgb(0xd88bb1)),
        "config" => Some(rgb(0xaab4c4)),
        "image" => Some(rgb(0xbb9af7)),
        "file" => Some(THEME.colors.subtle),
        _ => None,
    };
    // SVG elements are alpha masks. Use images for multicolor logos so
    // lettering and other overlapping color layers remain visible.
    match tint {
        Some(color) => svg()
            .path(asset)
            .size(THEME.icons.inline)
            .flex_none()
            .text_color(color)
            .into_any_element(),
        None => img(asset)
            .size(THEME.icons.inline)
            .flex_none()
            .into_any_element(),
    }
}

fn classify(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match name.as_str() {
        "dockerfile" | "containerfile" => return "docker",
        "cargo.toml" | "cargo.lock" => return "rust",
        "package.json" | "package-lock.json" => return "javascript",
        "tsconfig.json" => return "typescript",
        ".gitignore" | ".gitattributes" | ".gitmodules" => return "git",
        "makefile" | "justfile" | ".editorconfig" | ".env" => return "config",
        _ if name.starts_with(".env.") => return "config",
        _ => {}
    }
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match extension {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" | "tsx" => "react",
        "py" | "pyi" => "python",
        "go" | "mod" | "sum" => "go",
        "nix" => "nixos",
        "html" | "htm" => "html5",
        "css" => "css3",
        "scss" | "sass" => "sass",
        "vue" => "vuejs",
        "svelte" => "svelte",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "cplusplus",
        "cs" => "csharp",
        "java" => "java",
        "rb" | "gemspec" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "md" | "mdx" | "markdown" => "markdown",
        "json" | "jsonc" | "jsonl" => "json",
        "yaml" | "yml" => "yaml",
        "toml" | "ini" | "conf" | "lock" => "config",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" => "image",
        _ => "file",
    }
}

#[cfg(test)]
mod tests {
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
            ("src.rs/unknown", "file"),
            ("unknown.xyz", "file"),
            ("", "file"),
        ] {
            assert_eq!(classify(Path::new(path)), expected, "{path}");
        }
    }
}
