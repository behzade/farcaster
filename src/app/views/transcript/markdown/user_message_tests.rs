use gpui::AppContext as _;

use super::*;
use crate::app::views::transcript::markdown::{MarkdownStateKey, TranscriptMarkdownCache};

#[gpui::test]
fn user_messages_keep_line_breaks_and_generic_types(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cache = TranscriptMarkdownCache::default();
    for (index, source) in [
        "upstream/ghostty/    # unchanged files\npatches/ghostty/     # our changes\nshim/               # C adapters\nsrc/                # Rust code",
        "Vec<T> and Result<Option<T>, E>\nfn identity<T>(value: T) -> T",
        "<T>\nnext line",
        "<div title=\"**bold** &amp;\">literal</div>",
        "فارسی Vec<T>\r\nsecond line\rthird line",
    ].into_iter().enumerate() {
        cx.update(|cx| {
            let expected = source.replace("\r\n", "\n").replace('\r', "\n");
            for key in [
                MarkdownStateKey::item(index, 0),
                MarkdownStateKey::message_chunk(index, 1, 0),
                MarkdownStateKey::stream_chunk(index, 1, 0),
            ] {
                let state = cache.state(key, source, true, cx);
                assert_eq!(state.read(cx).rendered_text().as_str().trim_end(), expected, "{source}");
                state.update(cx, |state, cx| state.select_all(cx));
                assert_eq!(state.read(cx).selected_text().trim_end(), expected);
            }
        });
    }
}

#[gpui::test]
fn user_markdown_keeps_formatting_code_and_links(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let source = concat!(
        "# Heading\n\n**bold** and `Vec<T>`\nnext line\n\n",
        "```rust\nlet x: Vec<T> = vec![];\n```\n\n",
        "[file](</tmp/source.rs:12>) and <https://example.com>\n\n",
        "- one\n- two\n\n| A | B |\n| - | - |\n| C | D |"
    );
    cx.update(|cx| {
        let transformed = markdown(source);
        let parsed = markdown::to_mdast(&transformed, &markdown::ParseOptions::gfm()).unwrap();
        let original = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
        fn structural_nodes(node: &Node) -> Vec<String> {
            let mut nodes = match node {
                Node::Code(code) => vec![format!("code:{:?}:{}", code.lang, code.value)],
                Node::InlineCode(code) => vec![format!("inline:{}", code.value)],
                Node::Link(link) => vec![format!("link:{}", link.url)],
                Node::Heading(heading) => vec![format!("heading:{}", heading.depth)],
                Node::Strong(_) => vec!["strong".into()],
                Node::List(_) => vec!["list".into()],
                Node::Table(_) => vec!["table".into()],
                _ => Vec::new(),
            };
            if let Some(children) = node.children() {
                nodes.extend(children.iter().flat_map(structural_nodes));
            }
            nodes
        }
        assert_eq!(structural_nodes(&parsed), structural_nodes(&original));
        let state = cx.new(|cx| gpui_component::text::TextViewState::markdown(&transformed, cx));
        let rendered = state.read(cx).rendered_text();
        assert!(rendered.as_str().contains("bold and Vec<T>\nnext line"));
        assert!(rendered.as_str().contains("let x: Vec<T> = vec![];"));
    });
}

#[gpui::test]
fn user_rendering_does_not_change_assistant_markdown_or_cache_entries(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
        let cache = TranscriptMarkdownCache::default();
        let key = MarkdownStateKey::item(0, 0);
        let source = "one\ntwo Vec<T>";
        let assistant = cache.state(key, source, false, cx);
        let user = cache.state(key, source, true, cx);
        assert_eq!(
            assistant.read(cx).rendered_text().as_str().trim_end(),
            "one two Vec"
        );
        assert_eq!(user.read(cx).rendered_text().as_str().trim_end(), source);
        assert_eq!(cache.state(key, source, true, cx), user);
        assert_eq!(cache.state(key, source, false, cx), assistant);
    });
}

#[test]
fn code_and_existing_breaks_need_no_rewrite() {
    for source in [
        "plain text",
        "`Vec<T>`",
        "```rust\nVec<T>\n```",
        "    Vec<T>\n    Vec<U>",
        "one  \ntwo",
        "one\\\ntwo",
        "[file](</tmp/source.rs>)",
    ] {
        assert!(matches!(markdown(source), Cow::Borrowed(_)), "{source}");
    }
}

#[gpui::test]
fn literal_html_keeps_markdown_containers(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    for source in ["> <T>\n> next line", "- <T>\n  next line"] {
        cx.update(|cx| {
            let transformed = markdown(source);
            let state =
                cx.new(|cx| gpui_component::text::TextViewState::markdown(&transformed, cx));
            assert!(
                state
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .contains("<T>\nnext line"),
                "{}",
                state.read(cx).rendered_text().as_str()
            );
        });
    }
}
