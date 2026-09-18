use std::{
    borrow::Cow,
    collections::HashMap,
    path::Path,
    sync::{Arc, LazyLock, Mutex, PoisonError},
};

use gpui::{AnyElement, Image, ImageFormat, IntoElement as _, Rgba, Styled as _, img};

use super::theme::{color_hex, parse_hex, theme, theme_tint};

macro_rules! file_assets {
    ($($name:literal),* $(,)?) => {
        pub(crate) const ICON_NAMES: &[&str] = &[$($name),*];

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

static NATIVE_COLORS: LazyLock<Vec<Rgba>> = LazyLock::new(|| {
    ASSETS
        .iter()
        .map(|(_, bytes)| {
            std::str::from_utf8(bytes)
                .ok()
                .and_then(brand_color)
                .unwrap_or_else(|| theme().colors.text)
        })
        .collect()
});

type ImageCache = LazyLock<Mutex<HashMap<(usize, String), Arc<Image>>>>;

static IMAGES: ImageCache = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| Cow::Borrowed(*bytes))
}

pub(crate) fn file_icon(path: &Path) -> AnyElement {
    let index = index_of(classify(path)).unwrap_or_default();
    img(image(index))
        .size(theme().icons.inline)
        .flex_none()
        .into_any_element()
}

pub(crate) fn native_color(index: usize) -> Rgba {
    NATIVE_COLORS
        .get(index)
        .copied()
        .unwrap_or_else(|| theme().colors.text)
}

pub(crate) fn token_name(index: usize) -> Option<String> {
    ICON_NAMES.get(index).map(|name| format!("icon-{name}"))
}

fn image(index: usize) -> Arc<Image> {
    let tint = theme_tint(index);
    let key = (
        index,
        tint.map(color_hex).unwrap_or_else(|| "native".to_owned()),
    );
    let mut images = IMAGES.lock().unwrap_or_else(PoisonError::into_inner);
    images
        .entry(key)
        .or_insert_with(|| {
            let bytes = match tint {
                Some(color) => recolor(svg(index), color).into_bytes(),
                None => svg(index).as_bytes().to_vec(),
            };
            Arc::new(Image::from_bytes(ImageFormat::Svg, bytes))
        })
        .clone()
}

fn svg(index: usize) -> &'static str {
    ASSETS
        .get(index)
        .and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
        .unwrap_or_default()
}

fn recolor(svg: &str, color: Rgba) -> String {
    let hex = color_hex(Rgba { a: 1.0, ..color });
    let mut recolored = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find("fill=\"#") {
        let Some(end) = rest[start + 7..].find('"') else {
            break;
        };
        recolored.push_str(&rest[..start + 6]);
        recolored.push_str(&hex);
        rest = &rest[start + 7 + end..];
    }
    recolored.push_str(rest);
    recolored
}

fn brand_color(svg: &str) -> Option<Rgba> {
    let start = svg.find("fill=\"")? + 6;
    let end = svg[start..].find('"')?;
    parse_hex(&svg[start..start + end])
}

fn index_of(name: &str) -> Option<usize> {
    ICON_NAMES.iter().position(|candidate| *candidate == name)
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
#[path = "file_icons_tests.rs"]
mod tests;
