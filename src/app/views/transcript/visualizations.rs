use std::{
    borrow::Cow,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use url::Url;

const MARKER_START: &str = "\u{e200}visualize\u{e202}";
const MARKER_END: char = '\u{e201}';
const LINK_SCHEME: &str = "farcaster-visualize";

pub(crate) fn visualization_markdown(text: &str) -> Cow<'_, str> {
    let mut search_from = 0;
    let mut copied_from = 0;
    let mut markdown = String::new();

    while let Some(offset) = text[search_from..].find(MARKER_START) {
        let start = search_from + offset;
        let json_start = start + MARKER_START.len();
        let Some(end_offset) = text[json_start..].find(MARKER_END) else {
            break;
        };
        let json_end = json_start + end_offset;
        let marker_end = json_end + MARKER_END.len_utf8();
        search_from = marker_end;

        let Some((link, title)) = marker_link(&text[json_start..json_end]) else {
            continue;
        };
        markdown.push_str(&text[copied_from..start]);
        markdown.push_str("[Visualization: ");
        for character in title.chars() {
            if character.is_ascii_punctuation() {
                markdown.push('\\');
            }
            markdown.push(if character.is_control() {
                ' '
            } else {
                character
            });
        }
        markdown.push_str(" — Open in browser ↗](");
        markdown.push_str(link.as_str());
        markdown.push_str(" \"Open visualization\")");
        copied_from = marker_end;
    }

    if copied_from == 0 {
        Cow::Borrowed(text)
    } else {
        markdown.push_str(&text[copied_from..]);
        Cow::Owned(markdown)
    }
}

pub(super) fn visualization_target(link: &str) -> Option<PathBuf> {
    let path = visualization_path(link)?.canonicalize().ok()?;
    safe_visualization_path(&path).then_some(path)
}

pub(crate) fn is_visualization_link(link: &str) -> bool {
    Url::parse(link).is_ok_and(|link| link.scheme() == LINK_SCHEME)
}

fn marker_link(json: &str) -> Option<(Url, String)> {
    let marker = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let path = Path::new(marker.get("path")?.as_str()?);
    if !safe_visualization_path(path) {
        return None;
    }
    let file_url = Url::from_file_path(path).ok()?;
    let mut link = Url::parse(&format!("{LINK_SCHEME}://open")).expect("static URL is valid");
    link.query_pairs_mut()
        .append_pair("path", file_url.as_str());
    let title = marker
        .get("title")
        .and_then(|title| title.as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))?;
    Some((link, title.to_owned()))
}

/// Keep the source intact. The browser may load the output after this call returns,
/// so leave the unique preview file in the OS temporary directory.
pub(crate) fn prepare_visualization(link: &str) -> Result<Url, String> {
    let source = visualization_target(link)
        .ok_or_else(|| "Visualization is missing or has an unsafe path.".to_owned())?;
    const MAX_FRAGMENT_BYTES: u64 = 8 * 1024 * 1024;
    if !source.is_file() {
        return Err("Visualization must be an HTML file.".to_owned());
    }
    let mut fragment = String::new();
    std::fs::File::open(&source)
        .map_err(|error| error.to_string())?
        .take(MAX_FRAGMENT_BYTES + 1)
        .read_to_string(&mut fragment)
        .map_err(|error| error.to_string())?;
    if fragment.len() as u64 > MAX_FRAGMENT_BYTES {
        return Err("Visualization exceeds the 8 MiB preview limit.".to_owned());
    }
    let title = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Visualization");
    let document = visualization_document(&fragment, title);
    let mut output = tempfile::Builder::new()
        .prefix("farcaster-visualization-")
        .suffix(".html")
        .tempfile()
        .map_err(|error| error.to_string())?;
    output
        .write_all(document.as_bytes())
        .map_err(|error| error.to_string())?;
    let (_, path) = output.keep().map_err(|error| error.to_string())?;
    Url::from_file_path(path).map_err(|_| "Could not open visualization preview.".to_owned())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn visualization_document(fragment: &str, title: &str) -> String {
    // Bundled visualize 1.0.20 assets: no dependency on an installed skill at runtime.
    let css = include_str!("../../../../assets/visualize/visualize.css");
    let kit = include_str!("../../../../assets/visualize/visualize.html").replacen(
        "<!--__INLINE_VISUALIZATION_FRAGMENT__-->",
        fragment,
        1,
    );
    let resources = "blob: data: https://cdnjs.cloudflare.com https://cdn.jsdelivr.net https://esm.sh https://fonts.bunny.net https://fonts.googleapis.com https://fonts.gstatic.com https://unpkg.com";
    let policy = format!(
        "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' {resources}; style-src 'unsafe-inline' {resources}; img-src {resources}; font-src {resources}; media-src {resources}; worker-src blob:; connect-src blob: data:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
    );
    let title = escape_html(title);
    let frame = format!(
        r#"<!doctype html><html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta http-equiv="Content-Security-Policy" content="{policy}">
<title>{title}</title><style>{css}</style></head><body>{kit}</body></html>"#
    );
    let frame = escape_html(&frame);
    let shell_policy = policy.replace("frame-src 'none'", "frame-src 'self'");
    format!(
        r#"<!doctype html><html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<meta http-equiv="Content-Security-Policy" content="{shell_policy}">
<title>{title}</title><style>
:root {{ color-scheme: light dark; background: light-dark(#fff, #181818); }}
html, body {{ margin: 0; }}
body {{ padding: 1rem; }}
iframe {{ display: block; width: 100%; height: calc(100dvh - 2rem); border: 0; }}
</style></head><body><iframe sandbox="allow-scripts" referrerpolicy="no-referrer"
title="{title}" srcdoc="{frame}"></iframe></body></html>"#
    )
}

fn visualization_path(link: &str) -> Option<PathBuf> {
    let link = Url::parse(link).ok()?;
    if link.scheme() != LINK_SCHEME
        || link.host_str() != Some("open")
        || !matches!(link.path(), "" | "/")
        || !link.username().is_empty()
        || link.password().is_some()
        || link.port().is_some()
        || link.fragment().is_some()
    {
        return None;
    }
    let mut pairs = link.query_pairs();
    let (key, value) = pairs.next()?;
    if key != "path" || pairs.next().is_some() {
        return None;
    }
    let file_url = Url::parse(&value).ok()?;
    if file_url.scheme() != "file"
        || file_url.host_str().is_some()
        || !file_url.username().is_empty()
        || file_url.password().is_some()
        || file_url.port().is_some()
        || file_url.query().is_some()
        || file_url.fragment().is_some()
    {
        return None;
    }
    let path = file_url.to_file_path().ok()?;
    safe_visualization_path(&path).then_some(path)
}

fn safe_visualization_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some(extension) if extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        )
        && temporary_roots().iter().any(|root| path.starts_with(root))
}

fn temporary_roots() -> Vec<PathBuf> {
    let temporary = std::env::temp_dir();
    let mut roots = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        temporary.clone(),
    ];
    if let Ok(relative) = temporary.strip_prefix("/var") {
        roots.push(Path::new("/private/var").join(relative));
    }
    if let Ok(relative) = temporary.strip_prefix("/private/var") {
        roots.push(Path::new("/var").join(relative));
    }
    roots
}
