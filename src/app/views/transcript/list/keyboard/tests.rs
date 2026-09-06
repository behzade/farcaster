use super::*;
use gpui::{
    AppContext as _, Context, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, div, size,
};

fn text_row(text: &str) -> TextRow {
    let mut x = 0;
    let mut y = 0;
    TextRow {
        cells: text
            .graphemes(true)
            .map(|text| {
                let cell = Cell {
                    text: text.into(),
                    bounds: Bounds::new(
                        point(px(x as f32 * 10.0), px(y as f32 * 20.0)),
                        size(px(10.0), px(20.0)),
                    ),
                };
                if text == "\n" {
                    x = 0;
                    y += 1;
                } else {
                    x += 1;
                }
                cell
            })
            .collect(),
    }
}

#[test]
fn motions_and_inclusive_copy_preserve_graphemes_and_reverse_ranges() {
    let mut keyboard = Keyboard::default();
    let mut load = |row| text_row(["a e\u{301} 👩🏽‍💻!\nlong line", "next word"][row]);
    keyboard.row(0, &mut load);
    let first = Position { row: 0, cell: 0 };
    let word = keyboard.word(first, true, 2, &mut load);
    assert_eq!(word.cell, 2);
    let emoji = keyboard.word(word, true, 2, &mut load);
    assert_eq!(emoji.cell, 4);
    assert_eq!(keyboard.word(emoji, false, 2, &mut load), word);
    keyboard.anchor = Some(emoji);
    keyboard.cursor = Some(word);
    assert_eq!(keyboard.copy(&mut load).as_deref(), Some("e\u{301} 👩🏽‍💻"));
    keyboard.linewise = true;
    assert_eq!(
        keyboard.copy(&mut load).as_deref(),
        Some("a e\u{301} 👩🏽‍💻!\n")
    );
    let second_line = keyboard.vertical(word, true, 2, &mut load);
    assert_eq!(keyboard.cache[&0].cells[second_line.cell].text, "n");
    assert_eq!(keyboard.vertical(second_line, false, 2, &mut load), word);
}

#[test]
fn append_preserves_anchors_but_replacing_selected_content_and_blur_cancel() {
    let mut keyboard = Keyboard {
        active: true,
        cursor: Some(Position { row: 1, cell: 2 }),
        anchor: Some(Position { row: 0, cell: 1 }),
        ..Default::default()
    };
    keyboard.invalidate(2..2, true);
    assert_eq!(keyboard.mode(), "VISUAL");
    keyboard.invalidate(1..2, false);
    assert_eq!(keyboard.mode(), "NORMAL");
    assert!(keyboard.cursor.is_none());
    keyboard.pending.push_back(KeyboardCommand::Yank);
    keyboard.set_active(false);
    assert!(keyboard.pending.is_empty());
}

struct TextList {
    state: TranscriptListState,
    texts: Vec<String>,
    markdown: bool,
}

impl Render for TextList {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let texts = self.texts.clone();
        let markdown = self.markdown;
        transcript_list_grouped(
            self.state.clone(),
            |row| row,
            |_| panic!("keyboard copy must use rendered text"),
            move |index, _, _| {
                let content = if markdown {
                    gpui_component::text::TextView::markdown(
                        ("keyboard-markdown", index),
                        texts[index].clone(),
                    )
                    .into_any_element()
                } else {
                    div().child(texts[index].clone()).into_any_element()
                };
                div()
                    .w_full()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .child(content)
                    .into_any_element()
            },
        )
    }
}

fn draw(
    cx: &mut VisualTestContext,
    state: &TranscriptListState,
    texts: &[String],
    width: f32,
    markdown: bool,
) {
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(width), px(80.0)),
        |_, cx| {
            cx.new(|_| TextList {
                state: state.clone(),
                texts: texts.to_vec(),
                markdown,
            })
            .into_any_element()
        },
    );
}

#[gpui::test]
fn virtualized_yank_loads_unvisited_rows_and_end_only_follows_outside_visual(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let mut texts = (0..100)
        .map(|i| format!("row {i}: e\u{301} 👩🏽‍💻"))
        .collect::<Vec<_>>();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(20.0), texts.len()));
    state.set_keyboard_active(true);
    draw(cx, &state, &texts, 240.0, false);
    assert!(state.0.borrow().keyboard.cache.len() < texts.len());
    for command in [
        KeyboardCommand::Start,
        KeyboardCommand::Visual(false),
        KeyboardCommand::End,
    ] {
        state.keyboard_command(command);
    }
    draw(cx, &state, &texts, 240.0, false);
    assert_eq!(state.keyboard_mode(), "VISUAL");
    assert!(!state.is_following_tail());
    assert!(state.logical_scroll_top().item_ix > 90);
    for (command, mode) in [
        (KeyboardCommand::Copy, "VISUAL"),
        (KeyboardCommand::Yank, "NORMAL"),
    ] {
        state.keyboard_command(command);
        draw(cx, &state, &texts, 240.0, false);
        assert_eq!(state.keyboard_mode(), mode);
        assert_eq!(
            cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
            texts.join("\n\n")
        );
    }
    state.keyboard_command(KeyboardCommand::End);
    draw(cx, &state, &texts, 240.0, false);
    assert!(state.is_following_tail());
    texts.push("new streamed row".into());
    state.splice_with_size_hints(100..100, [px(20.0)]);
    draw(cx, &state, &texts, 240.0, false);
    assert_eq!(state.0.borrow().keyboard.cursor.unwrap().row, 100);
    assert!(state.0.borrow().keyboard.cache.len() < 10);
    state.reset();
    assert!(state.0.borrow().keyboard.cursor.is_none());
    assert_eq!(state.keyboard_mode(), "NORMAL");
}

#[gpui::test]
fn caret_geometry_and_selection_survive_wrapping(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let texts = vec!["alpha beta gamma delta epsilon zeta eta theta".into()];
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(100.0)]);
    state.keyboard_command(KeyboardCommand::Start);
    state.keyboard_command(KeyboardCommand::WordForward);
    state.keyboard_command(KeyboardCommand::Visual(false));
    draw(cx, &state, &texts, 240.0, false);
    let cursor = state.0.borrow().keyboard.cursor.unwrap();
    let wide = state.0.borrow().keyboard.cache[&0].cells[cursor.cell].bounds;
    draw(cx, &state, &texts, 50.0, false);
    let inner = state.0.borrow();
    assert_eq!(inner.keyboard.cursor, Some(cursor));
    assert_eq!(inner.keyboard.anchor, Some(cursor));
    let narrow = inner.keyboard.cache[&0].cells[cursor.cell].bounds;
    assert!(narrow.top() > wide.top());
    assert!(narrow.top() >= inner.scroll_y);
    assert!(narrow.bottom() <= inner.scroll_y + inner.viewport_height);
    drop(inner);
    state.keyboard_command(KeyboardCommand::Yank);
    draw(cx, &state, &texts, 50.0, false);
    assert_eq!(
        cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "b"
    );
}

#[gpui::test]
fn markdown_copy_uses_rendered_prose_and_code_not_markup(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let texts =
        vec!["**bold** and [link](https://example.com)\n\n```rust\nlet value = 2;\n```".into()];
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(200.0)]);
    for command in [
        KeyboardCommand::Start,
        KeyboardCommand::Visual(false),
        KeyboardCommand::End,
        KeyboardCommand::Yank,
    ] {
        state.keyboard_command(command);
    }
    draw(cx, &state, &texts, 240.0, true);
    let copied = cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains("bold and link"), "{copied:?}");
    assert!(copied.contains("let value = 2;"), "{copied:?}");
    assert!(
        !copied.contains("**") && !copied.contains("https://") && !copied.contains("```"),
        "{copied:?}"
    );
}

#[gpui::test]
fn caret_motion_at_viewport_end_does_not_resume_tail(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let texts = vec!["first line\nsecond line\nlast line".into()];
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(60.0)]);
    for command in [KeyboardCommand::Left, KeyboardCommand::Up] {
        state.keyboard_command(KeyboardCommand::End);
        draw(cx, &state, &texts, 240.0, false);
        assert!(state.is_following_tail());
        let end = state.0.borrow().keyboard.cursor.unwrap();

        state.keyboard_command(command);
        draw(cx, &state, &texts, 240.0, false);
        let moved = state.0.borrow().keyboard.cursor.unwrap();
        assert!(moved < end);
        assert_eq!(state.0.borrow().scroll_y, state.0.borrow().maximum_scroll());
        assert!(!state.is_following_tail());

        // Idle repaint used to snap the caret back to the end.
        draw(cx, &state, &texts, 240.0, false);
        assert_eq!(state.0.borrow().keyboard.cursor, Some(moved));
        assert!(!state.is_following_tail());
    }
}

#[gpui::test]
fn caret_hides_after_last_motion_but_stays_visible_in_visual_mode(cx: &mut TestAppContext) {
    use std::time::Duration;

    let cx = cx.add_empty_window();
    let texts = vec!["some text".into()];
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(20.0)]);
    state.set_keyboard_active(true);
    draw(cx, &state, &texts, 240.0, false);
    let visible = || state.0.borrow().keyboard.caret_visible();
    assert!(!visible());

    state.keyboard_command(KeyboardCommand::End);
    draw(cx, &state, &texts, 240.0, false);
    assert!(visible());
    cx.executor().advance_clock(Duration::from_secs(1));
    state.keyboard_command(KeyboardCommand::Left);
    draw(cx, &state, &texts, 240.0, false);
    cx.executor().advance_clock(Duration::from_secs(1));
    assert!(visible(), "new motion must replace the old timeout");
    cx.executor().advance_clock(Duration::from_secs(1));
    assert!(!visible());

    state.keyboard_command(KeyboardCommand::Visual(false));
    draw(cx, &state, &texts, 240.0, false);
    assert!(visible());
    state.keyboard_command(KeyboardCommand::Left);
    draw(cx, &state, &texts, 240.0, false);
    cx.executor().advance_clock(Duration::from_secs(2));
    assert!(visible(), "visual caret must outlive the motion timeout");
    state.keyboard_command(KeyboardCommand::Cancel);
    draw(cx, &state, &texts, 240.0, false);
    assert!(!visible());

    state.keyboard_command(KeyboardCommand::Right);
    draw(cx, &state, &texts, 240.0, false);
    assert!(visible());
    state.set_keyboard_active(false);
    state.set_keyboard_active(true);
    assert!(
        !visible(),
        "refocusing must not restore the old caret timer"
    );
}

fn apply(keyboard: &mut Keyboard, command: KeyboardCommand, texts: &[&str]) {
    keyboard.pending.push_back(command);
    let mut load = |row| text_row(texts[row]);
    if let Some(pos) = keyboard.cursor {
        keyboard.row(pos.row, &mut load);
    }
    keyboard.apply_pending(
        texts.len(),
        px(80.0),
        false,
        [ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        }; 3],
        &mut load,
    );
}

#[test]
fn word_and_word_end_motions_handle_punctuation_and_visual_anchors() {
    let texts = ["one.two  three\nnext"];
    let mut keyboard = Keyboard {
        cursor: Some(Position { row: 0, cell: 0 }),
        ..Default::default()
    };
    apply(&mut keyboard, KeyboardCommand::Visual(false), &texts);
    let anchor = keyboard.anchor;
    for (command, cell) in [
        (KeyboardCommand::WordEnd, 2),
        (KeyboardCommand::WordForward, 3),
        (KeyboardCommand::WordEnd, 6),
        (KeyboardCommand::PreviousWordEnd(false), 3),
        (KeyboardCommand::BigWordForward, 9),
        (KeyboardCommand::BigWordBackward, 0),
        (KeyboardCommand::BigWordEnd, 6),
        (KeyboardCommand::BigWordEnd, 13),
        (KeyboardCommand::PreviousWordEnd(true), 6),
    ] {
        apply(&mut keyboard, command, &texts);
        assert_eq!(keyboard.cursor.unwrap().cell, cell, "{command:?}");
        assert_eq!(keyboard.anchor, anchor);
    }
}

#[test]
fn line_find_repeat_and_bracket_motions_work_in_both_visual_modes() {
    for linewise in [false, true] {
        let texts = ["  a(b(c)d)e x x\nnext"];
        let mut keyboard = Keyboard {
            cursor: Some(Position { row: 0, cell: 2 }),
            ..Default::default()
        };
        apply(&mut keyboard, KeyboardCommand::Visual(linewise), &texts);
        let anchor = keyboard.anchor;
        for (command, cell) in [
            (KeyboardCommand::LineStart, 0),
            (KeyboardCommand::FirstNonblank, 2),
            (KeyboardCommand::MatchBracket, 9),
            (KeyboardCommand::MatchBracket, 3),
            (
                KeyboardCommand::Find {
                    character: 'x',
                    forward: true,
                    till: true,
                },
                11,
            ),
            (KeyboardCommand::RepeatFind(false), 13),
            (KeyboardCommand::RepeatFind(true), 13),
            (KeyboardCommand::LineEnd, 14),
        ] {
            apply(&mut keyboard, command, &texts);
            assert_eq!(keyboard.cursor.unwrap().cell, cell, "{command:?}");
            assert_eq!(keyboard.anchor, anchor);
        }
        apply(&mut keyboard, KeyboardCommand::SwapAnchor, &texts);
        assert_eq!(keyboard.cursor, anchor);
        assert_eq!(keyboard.anchor.unwrap().cell, 14);
    }
}

#[test]
fn search_wraps_across_virtualized_rows_and_preserves_visual_selection() {
    let texts = ["alpha beta", "other", "alpha last"];
    let mut keyboard = Keyboard {
        cursor: Some(Position { row: 0, cell: 0 }),
        ..Default::default()
    };
    apply(&mut keyboard, KeyboardCommand::Visual(false), &texts);
    apply(&mut keyboard, KeyboardCommand::SearchStart(true), &texts);
    for character in "alpha".chars() {
        apply(
            &mut keyboard,
            KeyboardCommand::SearchChar(character),
            &texts,
        );
    }
    apply(&mut keyboard, KeyboardCommand::SearchAccept, &texts);
    assert_eq!(keyboard.cursor, Some(Position { row: 2, cell: 0 }));
    assert_eq!(keyboard.anchor, Some(Position { row: 0, cell: 0 }));
    apply(&mut keyboard, KeyboardCommand::SearchNext(false), &texts);
    assert_eq!(keyboard.cursor, keyboard.anchor);
    apply(&mut keyboard, KeyboardCommand::SearchNext(true), &texts);
    assert_eq!(keyboard.cursor.unwrap().row, 2);
    apply(&mut keyboard, KeyboardCommand::SearchStart(false), &texts);
    apply(&mut keyboard, KeyboardCommand::SearchChar('z'), &texts);
    apply(&mut keyboard, KeyboardCommand::SearchCancel, &texts);
    assert_eq!(keyboard.search, "alpha");
    assert_eq!(keyboard.cursor.unwrap().row, 2);
}

#[test]
fn star_search_matches_whole_words_and_paragraph_sentence_motions_preserve_selection() {
    let texts = ["one. Two!\n\nthree", "oneword one"];
    let mut keyboard = Keyboard {
        cursor: Some(Position { row: 0, cell: 0 }),
        ..Default::default()
    };
    apply(&mut keyboard, KeyboardCommand::Visual(true), &texts);
    let anchor = keyboard.anchor;
    for (command, position) in [
        (
            KeyboardCommand::Sentence(true),
            Position { row: 0, cell: 5 },
        ),
        (
            KeyboardCommand::Paragraph(true),
            Position { row: 0, cell: 10 },
        ),
        (
            KeyboardCommand::Paragraph(false),
            Position { row: 0, cell: 0 },
        ),
        (
            KeyboardCommand::SearchWord(true),
            Position { row: 1, cell: 8 },
        ),
        (
            KeyboardCommand::SearchNext(false),
            Position { row: 0, cell: 0 },
        ),
    ] {
        apply(&mut keyboard, command, &texts);
        assert_eq!(keyboard.cursor, Some(position), "{command:?}");
        assert_eq!(keyboard.anchor, anchor);
    }
}

#[gpui::test]
fn logical_and_screen_motions_differ_on_wrapped_text(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let texts = vec!["alpha beta gamma delta epsilon\nsecond line".into()];
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(100.0)]);
    state.keyboard_command(KeyboardCommand::Start);
    state.keyboard_command(KeyboardCommand::Visual(false));
    draw(cx, &state, &texts, 100.0, false);
    let anchor = state.0.borrow().keyboard.anchor;
    state.keyboard_command(KeyboardCommand::ScreenDown);
    draw(cx, &state, &texts, 100.0, false);
    let wrapped = state.0.borrow().keyboard.cursor.unwrap();
    assert!(wrapped.cell > 0 && wrapped.cell < 30);
    state.keyboard_command(KeyboardCommand::Start);
    state.keyboard_command(KeyboardCommand::Down);
    draw(cx, &state, &texts, 100.0, false);
    assert_eq!(state.0.borrow().keyboard.cursor.unwrap().cell, 31);
    assert_eq!(state.0.borrow().keyboard.anchor, anchor);
    state.keyboard_command(KeyboardCommand::Up);
    draw(cx, &state, &texts, 100.0, false);
    assert_eq!(state.0.borrow().keyboard.cursor, anchor);
}

#[gpui::test]
fn viewport_alignment_preserves_caret_and_visual_anchor(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let texts = (0..20).map(|i| format!("row {i}")).collect::<Vec<_>>();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(20.0), texts.len()));
    state.keyboard_command(KeyboardCommand::Start);
    for _ in 0..10 {
        state.keyboard_command(KeyboardCommand::Down);
    }
    state.keyboard_command(KeyboardCommand::Visual(false));
    draw(cx, &state, &texts, 240.0, false);
    let cursor = state.0.borrow().keyboard.cursor;
    for alignment in [0, 1, 2] {
        state.keyboard_command(KeyboardCommand::Align(alignment));
        draw(cx, &state, &texts, 240.0, false);
        let inner = state.0.borrow();
        assert_eq!(inner.keyboard.cursor, cursor);
        assert_eq!(inner.keyboard.anchor, cursor);
        assert!(!inner.following_tail);
        let pos = cursor.unwrap();
        let cell = &inner.keyboard.cache[&pos.row].cells[pos.cell];
        let y = inner.heights.prefix(pos.row) + cell.bounds.top() - inner.scroll_y;
        assert_eq!(
            y,
            (inner.viewport_height - cell.bounds.size.height) * (alignment as f32 / 2.0)
        );
    }
}
