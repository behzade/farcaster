use super::*;

#[test]
fn reconstruct_markdown_wraps_marked_runs() {
    // "bold" fully covered by a bold mark.
    let marks = vec![(0..4, TextMark::default().bold())];
    assert_eq!(reconstruct_markdown("bold", &marks, 0..4), "**bold**");
    // Partial selection inside the bold run still wraps the slice.
    assert_eq!(reconstruct_markdown("bold", &marks, 1..3), "**ol**");
}

#[test]
fn reconstruct_markdown_emits_unmarked_text_verbatim() {
    // "a b c": plain, code, plain across three runs concatenated.
    let text = "a b c";
    let marks = vec![(2..3, TextMark::default().code())];
    assert_eq!(reconstruct_markdown(text, &marks, 0..5), "a `b` c");
    // Selecting only the plain tail.
    assert_eq!(reconstruct_markdown(text, &marks, 3..5), " c");
}

#[test]
fn reconstruct_markdown_handles_code_italic_strike_link() {
    assert_eq!(
        reconstruct_markdown("x", &[(0..1, TextMark::default().code())], 0..1),
        "`x`"
    );
    assert_eq!(
        reconstruct_markdown("x", &[(0..1, TextMark::default().italic())], 0..1),
        "*x*"
    );
    assert_eq!(
        reconstruct_markdown("x", &[(0..1, TextMark::default().strikethrough())], 0..1),
        "~~x~~"
    );
    let link = TextMark::default().link(LinkMark {
        url: "https://example.com".into(),
        ..Default::default()
    });
    assert_eq!(
        reconstruct_markdown("x", &[(0..1, link)], 0..1),
        "[x](https://example.com)"
    );
}

#[test]
fn reconstruct_markdown_nested_bold_italic() {
    // A single run marked both bold and italic (as produced by `**_x_**`).
    let mark = TextMark::default().bold().italic();
    // Inner (italic) is applied first, then bold: `***x***`.
    assert_eq!(reconstruct_markdown("x", &[(0..1, mark)], 0..1), "***x***");
}

/// Build a paragraph whose combined `state.text` is the concatenation of
/// its children (mirroring `Paragraph::render`), then set the paragraph
/// selection so `selected_source` can be exercised without a real paint.
fn paragraph_with_children(children: Vec<InlineNode>) -> Paragraph {
    let combined: String = children.iter().map(|c| c.text.to_string()).collect();
    let paragraph = Paragraph {
        span: None,
        children,
        link_refs: HashMap::new(),
        state: Arc::new(Mutex::new(InlineState::default())),
    };
    if let Ok(mut state) = paragraph.state.lock() {
        state.set_text(combined.into());
    }
    paragraph
}

fn set_paragraph_selection(paragraph: &Paragraph, range: Range<usize>) {
    if let Ok(mut state) = paragraph.state.lock() {
        state.selection = Some(range.into());
    }
}

#[test]
fn paragraph_selected_source_maps_partial_selection_across_runs() {
    // "This has **bold** text." rendered as ["This has ", "bold", " text."].
    let children = vec![
        InlineNode::new("This has ").marks(vec![(0..9, TextMark::default())]),
        InlineNode::new("bold").marks(vec![(0..4, TextMark::default().bold())]),
        InlineNode::new(" text.").marks(vec![(0..6, TextMark::default())]),
    ];
    let paragraph = paragraph_with_children(children);

    // Select the whole paragraph: "This has bold text." -> source with **.
    set_paragraph_selection(&paragraph, 0..(9 + 4 + 6));
    assert_eq!(paragraph.selected_source(), "This has **bold** text.");

    // Select only across the boundary "has **bold** te".
    // Rendered offsets: "has " starts at 5, "bold" at 9..13, " te" 13..16.
    set_paragraph_selection(&paragraph, 5..16);
    assert_eq!(paragraph.selected_source(), "has **bold** te");

    // Select entirely inside the bold run -> still wrapped.
    set_paragraph_selection(&paragraph, 10..12);
    assert_eq!(paragraph.selected_source(), "**ol**");
}

#[test]
fn paragraph_selected_source_matches_text_when_no_marks() {
    let children = vec![InlineNode::new("plain words").marks(vec![(0..11, TextMark::default())])];
    let paragraph = paragraph_with_children(children);
    set_paragraph_selection(&paragraph, 0..11);
    assert_eq!(paragraph.selected_source(), "plain words");
    assert_eq!(paragraph.selected_text(), "plain words");
}

fn selected_paragraph(text: &str) -> Paragraph {
    let len = text.len();
    let paragraph = paragraph_with_children(vec![
        InlineNode::new(text).marks(vec![(0..len, TextMark::default())]),
    ]);
    set_paragraph_selection(&paragraph, 0..len);
    paragraph
}

#[test]
fn heading_selected_source_prefixes_hashes() {
    let heading = BlockNode::Heading {
        level: 2,
        children: selected_paragraph("Title"),
        span: None,
    };
    assert_eq!(heading.selected_text(SelectionFormat::Source), "## Title\n");
    // Rendered text keeps no marker.
    assert_eq!(heading.selected_text(SelectionFormat::Plain), "Title\n");
}

#[test]
fn unordered_list_selected_source_prefixes_dash() {
    let list = BlockNode::List {
        ordered: false,
        start: 1,
        span: None,
        children: vec![
            BlockNode::ListItem {
                children: vec![BlockNode::Paragraph(selected_paragraph("one"))],
                spread: false,
                checked: None,
                span: None,
            },
            BlockNode::ListItem {
                children: vec![BlockNode::Paragraph(selected_paragraph("two"))],
                spread: false,
                checked: None,
                span: None,
            },
        ],
    };
    assert_eq!(
        list.selected_text(SelectionFormat::Source),
        "- one\n- two\n"
    );
}

#[test]
fn ordered_list_selected_source_preserves_start_and_unselected_item_offsets() {
    for (start, full, partial) in [
        (0, "0. first\n1. second\n", "1. second\n"),
        (1, "1. first\n2. second\n", "2. second\n"),
        (9, "9. first\n10. second\n", "10. second\n"),
    ] {
        let list = BlockNode::List {
            ordered: true,
            start,
            span: None,
            children: vec![
                BlockNode::ListItem {
                    children: vec![BlockNode::Paragraph(selected_paragraph("first"))],
                    spread: false,
                    checked: None,
                    span: None,
                },
                BlockNode::ListItem {
                    children: vec![BlockNode::Paragraph(selected_paragraph("second"))],
                    spread: false,
                    checked: None,
                    span: None,
                },
            ],
        };
        assert_eq!(list.selected_text(SelectionFormat::Source), full);
        if let BlockNode::List { children, .. } = &list {
            children[0].clear_selection();
        }
        assert_eq!(list.selected_text(SelectionFormat::Source), partial);
    }
}

#[test]
fn nested_list_selected_source_indents_sublists() {
    for (ordered, start, nested_ordered, nested_start, expected) in [
        (false, 1, false, 1, "- one\n  - nested\n- two\n"),
        (true, 9, true, 4, "9. one\n   4. nested\n10. two\n"),
    ] {
        let nested = BlockNode::List {
            ordered: nested_ordered,
            start: nested_start,
            span: None,
            children: vec![BlockNode::ListItem {
                children: vec![BlockNode::Paragraph(selected_paragraph("nested"))],
                spread: false,
                checked: None,
                span: None,
            }],
        };
        let list = BlockNode::List {
            ordered,
            start,
            span: None,
            children: vec![
                BlockNode::ListItem {
                    children: vec![BlockNode::Paragraph(selected_paragraph("one")), nested],
                    spread: false,
                    checked: None,
                    span: None,
                },
                BlockNode::ListItem {
                    children: vec![BlockNode::Paragraph(selected_paragraph("two"))],
                    spread: false,
                    checked: None,
                    span: None,
                },
            ],
        };
        assert_eq!(list.selected_text(SelectionFormat::Source), expected);
    }
}

#[test]
fn task_list_selected_source_restores_checkboxes() {
    let list = BlockNode::List {
        ordered: false,
        start: 1,
        span: None,
        children: vec![
            BlockNode::ListItem {
                children: vec![BlockNode::Paragraph(selected_paragraph("done"))],
                spread: false,
                checked: Some(true),
                span: None,
            },
            BlockNode::ListItem {
                children: vec![BlockNode::Paragraph(selected_paragraph("todo"))],
                spread: false,
                checked: Some(false),
                span: None,
            },
        ],
    };
    assert_eq!(
        list.selected_text(SelectionFormat::Source),
        "- [x] done\n- [ ] todo\n"
    );
}

#[test]
fn blockquote_selected_source_prefixes_gt() {
    let quote = BlockNode::Blockquote {
        span: None,
        children: vec![BlockNode::Paragraph(selected_paragraph("quoted text"))],
    };
    assert_eq!(
        quote.selected_text(SelectionFormat::Source),
        "> quoted text\n"
    );
}

#[test]
fn table_selected_source_pipes_cells_with_alignment_row() {
    let cell = |text: &str| TableCell {
        children: selected_paragraph(text),
        width: None,
    };
    let table = Table {
        children: vec![
            TableRow {
                children: vec![cell("Name"), cell("Age")],
            },
            TableRow {
                children: vec![cell("Alice"), cell("30")],
            },
        ],
        column_aligns: vec![ColumnumnAlign::Left, ColumnumnAlign::Right],
        span: None,
    };
    let block = BlockNode::Table(table);
    assert_eq!(
        block.selected_text(SelectionFormat::Source),
        "| Name | Age |\n| :-- | --: |\n| Alice | 30 |\n"
    );
}

fn image_paragraph(alt: &str, url: &str) -> Paragraph {
    let image = ImageNode {
        url: url.into(),
        alt: Some(alt.into()),
        ..Default::default()
    };
    Paragraph {
        span: None,
        children: vec![InlineNode::image(image)],
        link_refs: HashMap::new(),
        state: Arc::new(Mutex::new(InlineState::default())),
    }
}

/// Every mark round-trips, including the two Markdown has no plain syntax
/// for.
#[test]
fn marks_round_trip_through_reconstruction() {
    let wrap = |mark: TextMark| reconstruct_markdown("x", &[(0..1, mark)], 0..1);

    assert_eq!(wrap(TextMark::default().bold()), "**x**");
    assert_eq!(wrap(TextMark::default().italic()), "*x*");
    assert_eq!(wrap(TextMark::default().code()), "`x`");
    assert_eq!(wrap(TextMark::default().strikethrough()), "~~x~~");
    assert_eq!(
        wrap(TextMark::default().highlight(crate::yellow(200))),
        "==x=="
    );
    // No Markdown syntax for underline, so it keeps the tag it came from.
    assert_eq!(wrap(TextMark::default().underline()), "<u>x</u>");

    // A link keeps its title, which Markdown carries after the URL.
    assert_eq!(
        wrap(TextMark::default().link(LinkMark {
            url: "https://example.com".into(),
            title: Some("Tip".into()),
            ..Default::default()
        })),
        "[x](https://example.com \"Tip\")"
    );
}

/// A block the selection covers whole comes straight from the source, so it
/// keeps what the author wrote instead of a normalized reconstruction.
#[test]
fn document_selected_source_slices_covered_blocks_from_the_source() {
    use crate::text::document::ParsedDocument;

    // Preserve the original emphasis spelling and list source instead of
    // reconstructing the fully covered block from its children.
    let source = "start\n\n3. _one_\n4. two\n\n---\n\nend";
    let list = "3. _one_\n4. two";
    let list_start = source.find(list).unwrap();
    let rule_start = source.find("---").unwrap();

    let document = ParsedDocument {
        source: source.into(),
        blocks: vec![
            BlockNode::Paragraph(selected_paragraph("start")),
            BlockNode::List {
                ordered: true,
                start: 3,
                children: vec![],
                span: Some(Span {
                    start: list_start,
                    end: list_start + list.len(),
                }),
            },
            BlockNode::HorizontalRule {
                span: Some(Span {
                    start: rule_start,
                    end: rule_start + 3,
                }),
            },
            BlockNode::Paragraph(selected_paragraph("end")),
        ],
    };

    assert_eq!(
        document.selected_text(SelectionFormat::Source, None),
        "start\n\n3. _one_\n4. two\n\n---\n\nend"
    );
}

#[test]
fn document_selected_source_includes_enclosed_image() {
    use crate::text::document::ParsedDocument;

    // A standalone image between two selected paragraphs is covered by the
    // selection, so it is copied whole even though it holds no selection of
    // its own — straight out of the source the parser located it in.
    let source = "before\n\n![alt](https://example.com/i.png)\n\nafter";
    let image_markdown = "![alt](https://example.com/i.png)";
    let start = source.find(image_markdown).unwrap();
    let mut image = image_paragraph("alt", "https://example.com/i.png");
    image.span = Some(Span {
        start,
        end: start + image_markdown.len(),
    });

    let document = ParsedDocument {
        source: source.into(),
        blocks: vec![
            BlockNode::Paragraph(selected_paragraph("before")),
            BlockNode::Paragraph(image),
            BlockNode::Paragraph(selected_paragraph("after")),
        ],
    };
    assert_eq!(
        document.selected_text(SelectionFormat::Source, None),
        "before\n\n![alt](https://example.com/i.png)\n\nafter"
    );
}

#[test]
fn document_selected_source_drops_unenclosed_image() {
    use crate::text::document::ParsedDocument;

    // An image after the only selected block, with nothing selected after
    // it, is not enclosed and is dropped.
    let document = ParsedDocument {
        source: String::new().into(),
        blocks: vec![
            BlockNode::Paragraph(selected_paragraph("before")),
            BlockNode::Paragraph(image_paragraph("alt", "u")),
        ],
    };
    assert_eq!(
        document.selected_text(SelectionFormat::Source, None),
        "before"
    );
}

fn selected_code_block(code: &str, lang: Option<&str>) -> BlockNode {
    let block = CodeBlock::new(
        code.to_string().into(),
        lang.map(|l| l.to_string().into()),
        None::<Span>,
    );
    if let Ok(mut state) = block.state.lock() {
        let len = state.text.len();
        state.selection = Some((0..len).into());
    }
    BlockNode::CodeBlock(block)
}

#[test]
fn code_block_selected_source_wraps_in_fence_with_lang() {
    let block = selected_code_block("let x = 1;\n", Some("rust"));
    let code = block.selected_text(SelectionFormat::Plain);
    let code_trimmed = code.trim_end_matches('\n');
    // The source wraps the (trailing-newline-trimmed) selected code in a
    // fenced block carrying the language; the block arm adds one trailing
    // newline.
    assert_eq!(
        block.selected_text(SelectionFormat::Source),
        format!("```rust\n{}\n```\n", code_trimmed)
    );
    assert!(
        block
            .selected_text(SelectionFormat::Source)
            .starts_with("```rust\n")
    );
    assert!(
        block
            .selected_text(SelectionFormat::Source)
            .trim_end()
            .ends_with("\n```")
    );
}

#[test]
fn code_block_selected_source_without_lang() {
    let block = selected_code_block("plain\n", None);
    let code_trimmed = block.selected_text(SelectionFormat::Plain);
    let code_trimmed = code_trimmed.trim_end_matches('\n');
    assert_eq!(
        block.selected_text(SelectionFormat::Source),
        format!("```\n{}\n```\n", code_trimmed)
    );
}

#[test]
fn document_selected_source_joins_blocks_with_blank_line() {
    use crate::text::document::ParsedDocument;

    // A heading, a paragraph, and a two-item ordered list, each fully
    // selected. Top-level blocks must be separated by a blank line so the
    // copied Markdown re-renders with the same structure.
    let document = ParsedDocument {
        source: String::new().into(),
        blocks: vec![
            BlockNode::Heading {
                level: 1,
                children: selected_paragraph("Title"),
                span: None,
            },
            BlockNode::Paragraph(selected_paragraph("A paragraph.")),
            selected_code_block("let x = 1;\n", Some("rust")),
            BlockNode::List {
                ordered: true,
                start: 1,
                span: None,
                children: vec![
                    BlockNode::ListItem {
                        children: vec![BlockNode::Paragraph(selected_paragraph("one"))],
                        spread: false,
                        checked: None,
                        span: None,
                    },
                    BlockNode::ListItem {
                        children: vec![BlockNode::Paragraph(selected_paragraph("two"))],
                        spread: false,
                        checked: None,
                        span: None,
                    },
                ],
            },
        ],
    };

    assert_eq!(
        document.selected_text(SelectionFormat::Source, None),
        "# Title\n\nA paragraph.\n\n```rust\nlet x = 1;\n```\n\n1. one\n2. two"
    );
}

#[test]
fn code_block_equality_includes_code_content() {
    let first = CodeBlock::new("let value = 1;".into(), Some("rust".into()), None::<Span>);
    let second = CodeBlock::new("let value = 2;".into(), Some("rust".into()), None::<Span>);

    assert_ne!(first, second);
}
