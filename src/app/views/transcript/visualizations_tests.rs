use std::path::Path;

use super::visualizations::{
    is_visualization_link, prepare_visualization, visualization_markdown, visualization_target,
};

fn rendered_link(path: &Path) -> String {
    let payload = serde_json::json!({ "path": path.to_string_lossy() });
    let marker = format!("\u{e200}visualize\u{e202}{payload}\u{e201}");
    let markdown = visualization_markdown(&marker);
    markdown
        .split_once("](")
        .map(|(_, link)| link)
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
        "Before [Visualization: model picker\\.html — Open in browser ↗](farcaster-visualize://open?path=file%3A%2F%2F%2Fprivate%2Ftmp%2Fmodel%2520picker.html \"Open visualization\") after"
    );
}

#[test]
fn visualization_title_is_visible_and_cannot_inject_markdown() {
    let payload = serde_json::json!({"path": "/private/tmp/demo.html", "title": "[Demo](https://example.com)\n<strong>"});
    let marker = format!("\u{e200}visualize\u{e202}{payload}\u{e201}");
    let rendered = visualization_markdown(&marker);
    assert!(rendered.starts_with("[Visualization: \\[Demo\\]\\(https\\:\\/\\/example\\.com\\) \\<strong\\> — Open in browser ↗]("));
}

#[test]
fn preview_wraps_fragment_without_changing_source() {
    let source = tempfile::Builder::new().suffix(".html").tempfile().unwrap();
    let fragment = "<h1>Demo</h1><script>document.body.dataset.ready = 'yes';</script><!-- \"</iframe><script>parent.bad = true</script> -->";
    std::fs::write(source.path(), fragment).unwrap();
    let output = prepare_visualization(&rendered_link(source.path()))
        .unwrap()
        .to_file_path()
        .unwrap();
    assert_ne!(output, source.path());
    let document = std::fs::read_to_string(&output).unwrap();
    std::fs::remove_file(output).unwrap();
    assert_eq!(std::fs::read_to_string(source.path()).unwrap(), fragment);
    assert!(document.contains("sandbox=\"allow-scripts\""));
    assert!(!document.contains("allow-same-origin"));
    assert_eq!(document.matches("</iframe>").count(), 1);
    assert!(document.contains("--viz-series-1:"));
    assert!(document.contains("lucide@"));
    assert!(document.contains("&lt;h1&gt;Demo&lt;/h1&gt;"));
    assert!(document.contains("document.body.dataset.ready = &#39;yes&#39;;"));
}

#[test]
fn preview_rejects_missing_files_directories_and_large_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("demo.html");
    let link = rendered_link(&source);
    assert!(prepare_visualization(&link).is_err());
    std::fs::create_dir(&source).unwrap();
    assert!(prepare_visualization(&link).is_err());
    std::fs::remove_dir(&source).unwrap();
    std::fs::File::create(&source)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(prepare_visualization(&link).unwrap_err().contains("8 MiB"));
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
        Some(file.path().canonicalize().expect("resolve temporary file"))
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
