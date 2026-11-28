use super::*;

#[test]
fn file_rules_preserve_openers_and_explicit_source_lines() {
    use OpenAction::*;
    for (path, expected) in [
        ("IMAGE.PNG", PreviewImage(ImageFormat::Png)),
        ("photo.jpeg", PreviewImage(ImageFormat::Jpeg)),
        ("diagram.svg", PreviewImage(ImageFormat::Svg)),
        ("slides.html", Browser),
        ("slides.HTM", Browser),
        ("source.rs", Editor),
        ("notes.md", Editor),
        ("Dockerfile", Editor),
        (".env", Editor),
        ("document.pdf", System),
        ("recording.wav", System),
        ("archive.zip", System),
    ] {
        let rule = resolve(AttachmentSource::File(Path::new(path)));
        assert_eq!(rule.view, AttachmentView::File, "{path}");
        assert_eq!(rule.open_at(None), expected, "{path}");
        assert_eq!(
            rule.open_at(Some(12)),
            if expected == Browser {
                Editor
            } else {
                expected
            },
            "{path}"
        );
    }
}

#[test]
fn encoded_images_and_html_code_share_file_actions_with_their_own_views() {
    use OpenAction::*;
    let image = resolve(AttachmentSource::Image(ImageFormat::Png));
    let file = resolve(AttachmentSource::File(Path::new("image.png")));
    assert_eq!(image.view, AttachmentView::Image);
    assert_eq!(image.open, file.open);
    for rule in [image, file] {
        assert_eq!(
            rule.preview_action(IMAGE_PREVIEW_BYTES, true),
            PreviewImage(ImageFormat::Png)
        );
        assert_eq!(rule.preview_action(IMAGE_PREVIEW_BYTES + 1, true), System);
        assert_eq!(rule.preview_action(1, false), System);
    }
    let html = resolve(AttachmentSource::Code("HTML"));
    assert_eq!(html.view, AttachmentView::Code);
    assert_eq!(html.open, Browser);
    assert_eq!(html.source, Some(Editor));
    assert!(resolve(AttachmentSource::Code("rust")).source.is_none());

    for (url, expected) in [
        ("https://example.com/picture.png", Browser),
        ("http://localhost/slides.html", Browser),
        ("mailto:person@example.com", System),
        ("vscode://file/project/main.rs", Unavailable),
        ("not a URL", Unavailable),
    ] {
        let rule = resolve(AttachmentSource::Url(url));
        assert_eq!(rule.open, expected, "{url}");
        assert!(
            rule.source.is_none(),
            "remote attachments have no local source action"
        );
    }
}
