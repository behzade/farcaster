use std::path::Path;

use url::Url;

use super::visualizations::{is_visualization_link, visualization_markdown, visualization_target};

fn rendered_link(path: &Path) -> String {
    let payload = serde_json::json!({ "path": path.to_string_lossy() });
    let marker = format!("\u{e200}visualize\u{e202}{payload}\u{e201}");
    let markdown = visualization_markdown(&marker);
    markdown
        .strip_prefix("[↗](")
        .and_then(|link| link.strip_suffix(" \"Open visualization\")"))
        .expect("valid marker becomes a link")
        .to_owned()
}

#[test]
fn visualization_markers_become_explicit_links() {
    let marker = "\u{e200}visualize\u{e202}{\"path\":\"/private/tmp/model picker.html\",\"mode\":\"wide\"}\u{e201}";
    let input = format!("Before {marker} after");
    let rendered = visualization_markdown(&input);

    assert_eq!(
        rendered,
        "Before [↗](farcaster-visualize://open?path=file%3A%2F%2F%2Fprivate%2Ftmp%2Fmodel%2520picker.html \"Open visualization\") after"
    );
}

#[test]
fn unsafe_or_malformed_markers_stay_text() {
    for marker in [
        "\u{e200}visualize\u{e202}{\"path\":\"/etc/preview.html\"}\u{e201}",
        "\u{e200}visualize\u{e202}{\"path\":\"/private/tmp/../secret.html\"}\u{e201}",
        "\u{e200}visualize\u{e202}{\"path\":\"/private/tmp/preview.js\"}\u{e201}",
        "\u{e200}visualize\u{e202}not json\u{e201}",
    ] {
        assert_eq!(visualization_markdown(marker), marker);
    }
}

#[test]
fn visualization_target_resolves_only_one_safe_temporary_html_file() {
    let file = tempfile::Builder::new()
        .suffix(".html")
        .tempfile()
        .expect("create temporary visualization");
    let link = rendered_link(file.path());

    assert_eq!(
        visualization_target(&link),
        Url::from_file_path(file.path().canonicalize().expect("resolve temporary file")).ok()
    );
    assert_eq!(visualization_target(&format!("{link}&other=value")), None);
    assert_eq!(
        visualization_target("farcaster-visualize://open?path=file%3A%2F%2F%2Fetc%2Fpreview.html"),
        None
    );
    for link in [
        link.replacen(
            "farcaster-visualize://open",
            "farcaster-visualize://user@open",
            1,
        ),
        link.replacen(
            "farcaster-visualize://open",
            "farcaster-visualize://open:443",
            1,
        ),
        format!("{link}#fragment"),
        format!("{link}%23fragment"),
        format!("{link}%3Fquery"),
    ] {
        assert!(is_visualization_link(&link));
        assert_eq!(visualization_target(&link), None, "{link}");
    }
}
