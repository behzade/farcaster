use super::*;

#[test]
fn file_links_resolve_paths_and_source_lines() {
    for (link, expected, line) in [
        ("src/main.rs", "/project/src/main.rs", None),
        ("../shared.rs:12", "/shared.rs", Some(12)),
        (
            "/project/src/main.rs:12:3",
            "/project/src/main.rs",
            Some(12),
        ),
        ("src/main.rs#L12-L18", "/project/src/main.rs", Some(12)),
        ("file:///project/a%20b.rs#L4", "/project/a b.rs", Some(4)),
        ("docs/a%20b.md:7", "/project/docs/a b.md", Some(7)),
        ("README.md#heading", "/project/README.md", None),
        ("sandbox:/project/a%20b.png", "/project/a b.png", None),
    ] {
        assert_eq!(
            local_file_target(link, Path::new("/project")),
            Some((PathBuf::from(expected), line)),
            "{link}"
        );
    }
}

#[test]
fn embedded_images_keep_bytes_and_reuse_decoded_image() {
    let uri = "data:image/png;base64,iVBORw0KGgo=";
    let first = data_image(uri).unwrap();
    assert_eq!(first.bytes(), b"\x89PNG\r\n\x1a\n");
    assert!(Arc::ptr_eq(&first, &data_image(uri).unwrap()));
    assert!(data_image("data:text/html;base64,aGVsbG8=").is_none());
    assert!(data_image("data:image/png;base64,not-base64").is_none());
}

#[test]
fn external_links_and_anchors_are_not_editor_targets() {
    for link in [
        "https://example.com/file.rs#L12",
        "http://localhost:3000",
        "mailto:user@example.com",
        "vscode://file/project/main.rs:12",
        "//example.com/path",
        "file://remote-server/project/file.rs",
        "#heading",
        "",
    ] {
        assert_eq!(
            local_file_target(link, Path::new("/project")),
            None,
            "{link}"
        );
    }
}

#[test]
fn screenshot_headers_set_natural_bounds_even_when_extension_is_wrong() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("screenshot.png");
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(640, 160)
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    std::fs::write(&path, bytes.into_inner()).unwrap();
    assert_eq!(
        image_preview_dimensions(path.to_str().unwrap()),
        Some((640, 160))
    );
    assert_eq!(
        image_preview_dimensions("https://example.com/image.png"),
        None
    );
}
