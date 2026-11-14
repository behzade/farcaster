use super::RecentCache;

#[test]
fn images_gain_click_targets_without_changing_existing_links_or_code() {
    let source = "![local](a.png)\n\n![ref][image]\n\n[image]: a%20b.png\n\n[![linked](thumb.png)](large.png)\n\n```md\n![code](code.png)\n```";
    let rendered = super::clickable_images(source);
    assert!(rendered.contains("src=\"a.png\""));
    assert!(rendered.contains("src=\"a%20b.png\""));
    assert!(rendered.contains("src=\"thumb.png\""));
    assert!(rendered.contains("](large.png)"));
    assert!(rendered.contains("```md\n![code](code.png)\n```"));
}

#[test]
fn local_html_links_offer_source_but_explicit_source_links_do_not_repeat_it() {
    for path in ["slides.html", "slides.HTM"] {
        assert!(
            super::clickable_images(&format!("[slides]({path})"))
                .contains(&format!("[Edit source](<{path}#L1>)"))
        );
    }
    assert_eq!(
        super::clickable_images("[source](slides.html#L1)"),
        "[source](slides.html#L1)"
    );
    assert_eq!(
        super::clickable_images("[web](https://example.com/slides.html)"),
        "[web](https://example.com/slides.html)"
    );
}

#[test]
fn recently_used_markdown_state_survives_virtualization() {
    let mut cache = RecentCache::new(2);
    let mut parses = 0;
    let (first, first_hit) = cache.get_or_insert_with("final-row", || {
        parses += 1;
        "parsed state"
    });
    let _other = cache.get_or_insert_with("other-row", || "other state");
    let (restored, restored_hit) = cache.get_or_insert_with("final-row", || {
        parses += 1;
        "replacement state"
    });

    assert_eq!(first, restored);
    assert!(!first_hit);
    assert!(restored_hit);
    assert_eq!(parses, 1);
}

#[test]
fn captions_and_screenshots_on_separate_lines_form_separate_blocks() {
    use markdown::mdast::Node;
    let source = "Transcript:\n![Transcript](transcript.png)\n\nComposer:\n[![Composer](composer.png)](full.png)";
    let rendered = super::clickable_images(source);
    let root = markdown::to_mdast(&rendered, &markdown::ParseOptions::gfm()).unwrap();
    let children = root.children().unwrap();
    assert_eq!(
        children.len(),
        4,
        "each caption and screenshot is its own block"
    );
    for (index, label) in [(0, "Transcript:"), (2, "Composer:")] {
        let Node::Paragraph(paragraph) = &children[index] else {
            panic!("caption paragraph")
        };
        assert!(matches!(&paragraph.children[..], [Node::Text(text)] if text.value == label));
    }
    assert!(
        rendered.contains("](full.png)"),
        "keep the original image link target"
    );
    assert!(
        !rendered.contains("height="),
        "measure each screenshot's aspect ratio"
    );
}

#[test]
fn inline_images_and_list_images_keep_their_surrounding_structure() {
    let source = "Before ![icon](icon.png) after\n\n- Caption:\n  ![list](list.png)\n\n```md\nCaption:\n![example](example.png)\n```";
    let rendered = super::clickable_images(source);
    assert!(rendered.starts_with("Before [<img"));
    assert!(rendered.contains(">](<icon.png>) after"));
    assert!(rendered.contains("- Caption:\n  [<img"));
    assert!(rendered.ends_with("```md\nCaption:\n![example](example.png)\n```"));
}

#[gpui::test]
fn wide_screenshot_preview_uses_its_aspect_ratio(cx: &mut gpui::TestAppContext) {
    use gpui::{
        AppContext as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
        Styled as _, div, px,
    };
    use gpui_component::text::{TextView, TextViewState};
    struct Screenshot {
        state: gpui::Entity<TextViewState>,
    }
    impl Render for Screenshot {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().flex().flex_col().items_start().child(
                div()
                    .w(px(400.))
                    .debug_selector(|| "screenshot-document".into())
                    // Keep bytes pending to check the first frame, before
                    // the renderer can use a decoded image's dimensions.
                    .child(TextView::new(&self.state).image_source(|_| {
                        gpui::ImageSource::Custom(std::sync::Arc::new(|_, _| None))
                    })),
            )
        }
    }
    cx.update(gpui_component::init);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wide.png");
    image::DynamicImage::new_rgb8(640, 160).save(&path).unwrap();
    let (_, cx) = cx.add_window_view(|_, cx| {
        let source = format!("Caption:\n![wide screenshot]({})", path.display());
        let rendered = super::clickable_images(&source);
        Screenshot {
            state: cx.new(|cx| TextViewState::markdown(&rendered, cx)),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx.debug_bounds("screenshot-document").unwrap();
    assert!(
        bounds.size.height >= px(100.),
        "the screenshot must remain visible"
    );
    assert!(
        bounds.size.height < px(160.),
        "a 4:1 screenshot should not reserve a 16:9 box: {bounds:?}"
    );
}
