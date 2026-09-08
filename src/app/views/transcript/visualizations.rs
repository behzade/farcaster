use std::{
    borrow::Cow,
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

        let Some(link) = marker_link(&text[json_start..json_end]) else {
            continue;
        };
        markdown.push_str(&text[copied_from..start]);
        markdown.push_str("[↗](");
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

pub(crate) fn visualization_target(link: &str) -> Option<Url> {
    let path = visualization_path(link)?;
    let path = path.canonicalize().ok()?;
    if !safe_visualization_path(&path) {
        return None;
    }
    Url::from_file_path(path).ok()
}

pub(crate) fn is_visualization_link(link: &str) -> bool {
    Url::parse(link).is_ok_and(|link| link.scheme() == LINK_SCHEME)
}

fn marker_link(json: &str) -> Option<Url> {
    let marker = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let path = Path::new(marker.get("path")?.as_str()?);
    if !safe_visualization_path(path) {
        return None;
    }
    let file_url = Url::from_file_path(path).ok()?;
    let mut link = Url::parse(&format!("{LINK_SCHEME}://open")).expect("static URL is valid");
    link.query_pairs_mut()
        .append_pair("path", file_url.as_str());
    Some(link)
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
