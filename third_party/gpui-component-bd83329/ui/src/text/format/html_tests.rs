use gpui::{px, relative};

use crate::text::{
    document::ParsedDocument,
    node::{BlockNode, ImageNode, InlineNode, NodeContext, Paragraph},
};

use super::trim_text;

#[test]
fn test_cleanup_html() {
    let html = r#"<p>
        and
        <code>code</code>
        text
    </p>"#;
    let cleaned = super::cleanup_html(html);
    assert_eq!(
        String::from_utf8(cleaned).unwrap(),
        "<p>and <code>code</code> text"
    );

    let html = r#"<p>
        and
        <em>   <code>code</code>   <i>italic</i>   </em>
        text
    </p>"#;
    let cleaned = super::cleanup_html(html);
    assert_eq!(
        String::from_utf8(cleaned).unwrap(),
        "<p>and <em><code>code</code> <i>italic</i></em> text"
    );
}

#[test]
fn test_trim_text() {
    assert_eq!(trim_text("  \n\tHello world \t\r "), " Hello world ",);
}

#[test]
fn test_mark() {
    let mut cx = NodeContext::default();

    // `<mark>` is rendered as a highlight, kept as `==...==` in markdown.
    let html = r#"<p>Hello <mark>world</mark></p>"#;
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(node.to_markdown(), "Hello ==world==");

    let html = r#"<p><mark color="blue">blue</mark> and <mark style="background-color: #336699">hex</mark></p>"#;
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(node.to_markdown(), "==blue== and ==hex==");
}

#[test]
fn test_keep_spaces() {
    let html = r#"<p>and <code>code</code> text</p>"#;
    let mut cx = NodeContext::default();
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(node.to_markdown(), "and `code` text");

    let html = r#"
        <div>
        <p>
            and
            <em>   <code>code</code>   <i>italic</i>   </em>
            text
        </p>
        <p>
            <img src="https://example.com/image.png" alt="Example" width="100" height="200" title="Example Image" />
        </p>
        <ul>
            <li>Item 1</li>
            <li>Item 2
            </li>
        </ul>
        </div>
    "#;
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(
        node.to_markdown(),
        concat!(
            "and *code italic* text\n\n",
            "![Example](https://example.com/image.png \"Example Image\")\n\n",
            "- Item 1\n- Item 2",
        )
    );
}

#[test]
fn test_value_to_length() {
    assert_eq!(super::value_to_length("100px"), Some(px(100.).into()));
    assert_eq!(super::value_to_length("100%"), Some(relative(1.)));
    assert_eq!(super::value_to_length("56%"), Some(relative(0.56)));
    assert_eq!(super::value_to_length("240"), Some(px(240.).into()));
}

#[test]
fn test_image() {
    let html = r#"<img src="https://example.com/image.png" alt="Example" width="100" height="200" title="Example Image" />"#;
    let mut cx = NodeContext::default();
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(
        node,
        ParsedDocument {
            source: html.to_string().into(),
            blocks: vec![BlockNode::Paragraph(Paragraph {
                span: None,
                children: vec![InlineNode::image(ImageNode {
                    url: "https://example.com/image.png".to_string().into(),
                    alt: Some("Example".to_string().into()),
                    width: Some(px(100.).into()),
                    height: Some(px(200.).into()),
                    title: Some("Example Image".to_string().into()),
                    ..Default::default()
                })],
                ..Default::default()
            })]
        }
    );

    let html = r#"<img src="https://example.com/image.png" alt="Example" style="width: 80%" title="Example Image" />"#;
    let node = super::parse(html, &mut cx).unwrap();
    assert_eq!(
        node,
        ParsedDocument {
            source: html.to_string().into(),
            blocks: vec![BlockNode::Paragraph(Paragraph {
                span: None,
                children: vec![InlineNode::image(ImageNode {
                    url: "https://example.com/image.png".to_string().into(),
                    alt: Some("Example".to_string().into()),
                    width: Some(relative(0.8)),
                    height: None,
                    title: Some("Example Image".to_string().into()),
                    ..Default::default()
                })],
                ..Default::default()
            })]
        }
    );
}
