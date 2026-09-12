use super::{TextView, TextViewPlugin};
use crate::text::TextViewState;
use gpui::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Render, SharedString,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};

struct TextViewTestRoot {
    text_view: Entity<TextViewState>,
}

struct DummyTextViewPlugin;

impl TextViewPlugin for DummyTextViewPlugin {
    fn setup(self, mut text_view: TextView) -> TextView {
        text_view.selectable = true;
        text_view
    }
}

impl TextViewTestRoot {
    fn new(text: &str, cx: &mut Context<Self>) -> Self {
        let text = text.to_string();
        let text_view = cx.new(|cx| TextViewState::markdown(&text, cx));
        Self { text_view }
    }
}

impl Render for TextViewTestRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(160.))
            .child(
                div()
                    .h(px(24.))
                    .overflow_hidden()
                    .child(TextView::new(&self.text_view).selectable(true)),
            )
            .child(div().h(px(40.)).child("footer"))
    }
}

struct InlineImageTextViewTestRoot {
    text_view: Entity<TextViewState>,
}

impl InlineImageTextViewTestRoot {
    fn new(cx: &mut Context<Self>) -> Self {
        let text_view = cx.new(|cx| {
            TextViewState::markdown(
                "Build Status ![inline image](https://example.com/image.svg) after",
                cx,
            )
        });
        Self { text_view }
    }
}

impl Render for InlineImageTextViewTestRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(420.))
            .child(TextView::new(&self.text_view).selectable(true))
    }
}

#[gpui::test]
fn inline_image_keeps_surrounding_text_on_same_line(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (root, cx) = cx.add_window_view(|window, cx| {
        let content = cx.new(|cx| InlineImageTextViewTestRoot::new(cx));
        crate::Root::new(content, window, cx)
    });
    let content = root.read_with(cx, |root, _| {
        root.view()
            .clone()
            .downcast::<InlineImageTextViewTestRoot>()
            .unwrap()
    });
    let cx: &mut VisualTestContext = cx;

    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let inline_bounds = content.read_with(cx, |content, cx| {
        content.text_view.read(cx).selection_adapter.text_bounds()
    });

    assert_eq!(inline_bounds.len(), 2);
    assert_eq!(
        inline_bounds[0].top(),
        inline_bounds[1].top(),
        "text before and after an inline image should share a rendered line"
    );
    assert!(
        inline_bounds[1].left() - inline_bounds[0].right() > px(8.),
        "inline image should reserve horizontal space in the text layout"
    );
    assert!(
        inline_bounds[1].left() - inline_bounds[0].right() < px(40.),
        "unloaded inline image fallback should stay generic and compact"
    );
}

#[gpui::test]
fn inline_html_image_after_newline_does_not_panic(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, cx| {
        TextViewTestRoot::new(
            "Hi\n[<img src=\"https://example.com/image.svg\">](https://google.com/)",
            cx,
        )
    });
    let cx: &mut VisualTestContext = cx;

    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui::test]
fn list_item_renders_fenced_code_block_at_document_width(cx: &mut TestAppContext) {
    struct ListItemBlockRoot;

    impl Render for ListItemBlockRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(420.)).h(px(400.)).child(
                TextView::markdown(
                    "list-with-code",
                    "1. List item\n   ```rust\n   nested code\n   ```\n\n```rust\ntop-level code\n```",
                )
                .code_block_actions(|code_block, _, _| {
                    let selector = if code_block.code().contains("nested") {
                        "nested-code-action"
                    } else {
                        "top-level-code-action"
                    };
                    div()
                        .debug_selector(move || selector.into())
                        .child("Copy")
                })
                .scrollable(true)
                .p_5(),
            )
        }
    }

    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, _| ListItemBlockRoot);
    let cx: &mut VisualTestContext = cx;

    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let nested_action = cx.debug_bounds("nested-code-action").unwrap();
    let top_level_action = cx.debug_bounds("top-level-code-action").unwrap();
    assert!(
        top_level_action.right() - nested_action.right() < px(32.),
        "nested code block should fill the list item's available width"
    );
}

#[test]
fn plugin_accepts_text_view_plugins_beyond_markdown() {
    let view = TextView::markdown("plugin-test", "").plugin(DummyTextViewPlugin);

    assert!(view.selectable);
}

#[gpui::test]
fn clipped_markdown_link_does_not_open(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, cx| {
        TextViewTestRoot::new("visible\n\n[hidden](https://example.com)", cx)
    });
    let cx: &mut VisualTestContext = cx;

    cx.simulate_click(point(px(10.), px(34.)), Modifiers::default());

    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn markdown_link_opens_url_without_handler(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) =
        cx.add_window_view(|_, cx| TextViewTestRoot::new("[example](https://example.com)", cx));
    let cx: &mut VisualTestContext = cx;

    cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());

    assert_eq!(cx.opened_url(), Some("https://example.com".to_string()));
}

#[gpui::test]
fn right_click_does_not_open_url_without_handler(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) =
        cx.add_window_view(|_, cx| TextViewTestRoot::new("[example](https://example.com)", cx));
    let cx: &mut VisualTestContext = cx;

    cx.simulate_mouse_down(
        point(px(10.), px(10.)),
        MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(10.), px(10.)),
        MouseButton::Right,
        Modifiers::default(),
    );

    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn link_handler_receives_button_and_modifiers(cx: &mut TestAppContext) {
    use std::sync::{Arc, Mutex};

    struct LinkRoot {
        text_view: Entity<TextViewState>,
        clicks: Arc<Mutex<Vec<(SharedString, ClickEvent)>>>,
    }

    impl Render for LinkRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div()
                .w(px(240.))
                .child(
                    TextView::new(&self.text_view).on_link_click(move |url, event, _, _| {
                        clicks.lock().unwrap().push((url.clone(), event.clone()));
                    }),
                )
        }
    }

    cx.update(crate::init);
    let clicks = Arc::new(Mutex::new(Vec::new()));
    let captured = clicks.clone();
    let (_, cx) = cx.add_window_view(move |_, cx| LinkRoot {
        text_view: cx.new(|cx| TextViewState::markdown("[example](https://example.com)", cx)),
        clicks,
    });
    let cx: &mut VisualTestContext = cx;

    let mut modifiers = Modifiers::default();
    modifiers.control = true;
    cx.simulate_click(point(px(10.), px(10.)), modifiers);
    cx.simulate_mouse_down(
        point(px(10.), px(10.)),
        MouseButton::Middle,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(10.), px(10.)),
        MouseButton::Middle,
        Modifiers::default(),
    );
    cx.simulate_mouse_down(
        point(px(10.), px(10.)),
        MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(10.), px(10.)),
        MouseButton::Right,
        Modifiers::default(),
    );

    let clicks = captured.lock().unwrap();
    assert_eq!(clicks.len(), 3);
    assert_eq!(clicks[0].0, "https://example.com");
    assert!(!clicks[0].1.is_right_click() && !clicks[0].1.is_middle_click());
    assert!(clicks[0].1.modifiers().control);
    assert!(clicks[1].1.is_middle_click());
    assert!(clicks[2].1.is_right_click());
    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn linked_image_handler_receives_left_middle_and_right_clicks(cx: &mut TestAppContext) {
    use std::sync::{Arc, Mutex};

    struct LinkedImageRoot {
        text_view: Entity<TextViewState>,
        clicks: Arc<Mutex<Vec<(SharedString, ClickEvent)>>>,
    }

    impl Render for LinkedImageRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div().w(px(160.)).child(
                TextView::new(&self.text_view)
                    .selectable(true)
                    .on_link_click(move |url, event, _, _| {
                        clicks.lock().unwrap().push((url.clone(), event.clone()));
                    }),
            )
        }
    }

    cx.update(crate::init);
    let clicks = Arc::new(Mutex::new(Vec::new()));
    let captured = clicks.clone();
    let (root, cx) = cx.add_window_view(move |window, cx| {
        let content = cx.new(|cx| LinkedImageRoot {
            text_view: cx.new(|cx| {
                TextViewState::markdown(
                    r#"Before [<img src="https://example.com/image.svg" width="32" height="32">](https://example.com/image-link) after."#,
                    cx,
                )
            }),
            clicks,
        });
        crate::Root::new(content, window, cx)
    });
    let content = root.read_with(cx, |root, _| {
        root.view().clone().downcast::<LinkedImageRoot>().unwrap()
    });
    let cx: &mut VisualTestContext = cx;
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let inline_bounds = content.read_with(cx, |content, cx| {
        content.text_view.read(cx).selection_adapter.text_bounds()
    });
    assert!(
        inline_bounds.len() >= 2,
        "linked image needs text bounds on both sides: {inline_bounds:?}"
    );
    assert!(
        inline_bounds[1].left() - inline_bounds[0].right() >= px(24.),
        "linked image did not reserve the expected click target: {inline_bounds:?}"
    );
    let position = point(
        inline_bounds[0].right() + (inline_bounds[1].left() - inline_bounds[0].right()) * 0.5,
        inline_bounds[0].top() + px(8.),
    );
    for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
        cx.simulate_mouse_down(position, button, Modifiers::default());
        cx.simulate_mouse_up(position, button, Modifiers::default());
    }

    let clicks = captured.lock().unwrap();
    assert_eq!(clicks.len(), 3);
    assert!(
        clicks
            .iter()
            .all(|(url, _)| url == "https://example.com/image-link")
    );
    assert!(!clicks[0].1.is_right_click() && !clicks[0].1.is_middle_click());
    assert!(clicks[1].1.is_middle_click());
    assert!(clicks[2].1.is_right_click());
    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn non_focusable_text_preserves_focus_during_selection(cx: &mut TestAppContext) {
    struct SelectionRoot {
        owner: gpui::FocusHandle,
    }

    impl Render for SelectionRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(240.))
                .child(gpui_base::TextSelectionLayer)
                .child(
                    TextView::markdown("text", "select this text")
                        .selectable(true)
                        .focusable(false),
                )
                .child(div().track_focus(&self.owner).h(px(24.)))
        }
    }

    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        SelectionRoot { owner }
    });
    let cx: &mut VisualTestContext = cx;
    cx.simulate_mouse_down(
        point(px(10.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| assert!(view.read(cx).owner.is_focused(window)));
    cx.simulate_mouse_move(
        point(px(90.), px(10.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(90.), px(10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        assert!(view.read(cx).owner.is_focused(window));
        assert!(!gpui_base::TextSelection::selected_text(window, cx).is_empty());
    });
}

#[gpui::test]
fn clipped_markdown_cannot_start_selection(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (view, cx) =
        cx.add_window_view(|_, cx| TextViewTestRoot::new("visible\n\nhidden selection text", cx));
    let cx: &mut VisualTestContext = cx;

    cx.simulate_mouse_down(
        point(px(10.), px(34.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(90.), px(34.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(90.), px(34.)),
        MouseButton::Left,
        Modifiers::default(),
    );

    let selected_text = view.read_with(cx, |root, cx| root.text_view.read(cx).selected_text());
    assert!(
        selected_text.is_empty(),
        "unexpected selection: {selected_text:?}"
    );
}

/// A tall selectable TextView clipped by a short `overflow_hidden` viewport,
/// with a large blank footer below so a drag can extend the selection band
/// past the bottom of the clip while still proxy-anchoring to the view.
struct ClippedTallTextViewTestRoot {
    text_view: Entity<TextViewState>,
}

impl ClippedTallTextViewTestRoot {
    fn new(cx: &mut Context<Self>) -> Self {
        // Four separate blocks; only the first (and maybe part of the
        // second) fit inside the 40px clip. "charlie"/"delta" render well
        // below it.
        let text_view =
            cx.new(|cx| TextViewState::markdown("alpha\n\nbravo\n\ncharlie\n\ndelta", cx));
        Self { text_view }
    }
}

impl Render for ClippedTallTextViewTestRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(200.))
            .child(
                div()
                    .h(px(40.))
                    .overflow_hidden()
                    .child(TextView::new(&self.text_view).selectable(true)),
            )
            // A tall blank footer so a drag can reach a y below the clipped
            // text; a press there proxy-anchors to the TextView above.
            .child(div().h(px(160.)))
    }
}

/// Regression for copying a selection taller than the visible viewport.
///
/// The selection band runs from visible text at the top down to a point
/// far below the clip. Every glyph of the painted TextView is laid out even
/// though the lower ones are clipped away, so the copied text must include
/// the clipped-out "charlie"/"delta" — not just what is on screen. This
/// guards against re-adding a `content_mask` gate in
/// `Inline::layout_selections`.
#[gpui::test]
fn selection_band_beyond_clip_copies_offscreen_text(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let content = cx.new(ClippedTallTextViewTestRoot::new);
        crate::Root::new(content, window, cx)
    });
    let content = view.read_with(cx, |root, _| {
        root.view()
            .clone()
            .downcast::<ClippedTallTextViewTestRoot>()
            .unwrap()
    });
    let cx: &mut VisualTestContext = cx;

    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    // Anchor on visible text near the top, then drag to a point well below
    // the 40px clip (into the blank footer) and to the far right so the
    // last line is fully covered.
    cx.simulate_mouse_down(
        point(px(2.), px(8.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_move(
        point(px(180.), px(150.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(
        point(px(180.), px(150.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let selected_text = content.read_with(cx, |root, cx| root.text_view.read(cx).selected_text());
    assert!(
        selected_text.contains("delta"),
        "clipped-out text was not copied: {selected_text:?}"
    );
    assert!(
        selected_text.contains("charlie"),
        "clipped-out text was not copied: {selected_text:?}"
    );
}

#[gpui::test]
fn double_click_selects_word(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|_, cx| TextViewTestRoot::new("quick select value", cx));

    let cx: &mut VisualTestContext = cx;
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let position = point(px(10.), px(16.));
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let selected_text = view.read_with(cx, |root, cx| root.text_view.read(cx).selected_text());
    assert_eq!(selected_text.trim(), "quick");
}

#[gpui::test]
fn triple_click_selects_paragraph(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|_, cx| TextViewTestRoot::new("quick select value", cx));

    let cx: &mut VisualTestContext = cx;
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let position = point(px(10.), px(10.));
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 3,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 3,
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let selected_text = view.read_with(cx, |root, cx| root.text_view.read(cx).selected_text());
    assert_eq!(selected_text.trim(), "quick select value");
}

// Regression: markdown `TextView` items inside an outer `gpui::list` with
// `measure_all` must keep a stable total content height while scrolling.
// Before synchronous full-replace parsing, off-screen markdown views were
// first measured with empty content and the scrollbar thumb jittered as the
// total height grew during scrolling.
#[gpui::test]
fn outer_list_content_total_stable_while_scrolling(cx: &mut TestAppContext) {
    use gpui::{ListAlignment, ListState, list};

    const ITEMS: &[&str] = &[
        "# Heading\n\nA paragraph long enough to wrap across several lines and produce a non-trivial height.",
        "Short.",
        "Paragraph A\n\nParagraph B\n\nParagraph C with more words to increase the height.",
        "## Subheading\n\n- One\n- Two\n- Three\n\nClosing paragraph.",
        "Only one line.",
        "**Bold**: medium length text with `code` mixed with regular words.",
        "1. First\n2. Second\n3. Third\n\nA short closing paragraph.",
        "A long message with enough words to wrap across multiple lines, create a taller item, and verify that off-screen measurement matches visible measurement.",
    ];
    let n = 40usize;

    struct ListRoot {
        state: ListState,
    }
    impl Render for ListRoot {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(360.)).h(px(500.)).child(
                list(self.state.clone(), |ix, _w, _cx| {
                    div()
                        .w_full()
                        .child(TextView::markdown(
                            ("md", ix as u64),
                            ITEMS[ix % ITEMS.len()],
                        ))
                        .into_any_element()
                })
                .size_full(),
            )
        }
    }

    cx.update(crate::init);
    let state = ListState::new(n, ListAlignment::Top, px(2048.)).measure_all();
    let probe = state.clone();
    let (_view, cx) = cx.add_window_view(|_w, _cx| ListRoot { state });
    let cx: &mut VisualTestContext = cx;

    cx.run_until_parked();
    cx.update(|w, cx| {
        let _ = w.draw(cx);
    });
    cx.run_until_parked();
    cx.update(|w, cx| {
        let _ = w.draw(cx);
    });

    let total =
        |p: &ListState| f32::from(p.max_offset_for_scrollbar().y + p.viewport_bounds().size.height);
    let mut totals = vec![total(&probe)];
    for _ in 0..20 {
        probe.scroll_by(px(150.));
        cx.update(|w, cx| {
            let _ = w.draw(cx);
        });
        cx.run_until_parked();
        totals.push(total(&probe));
    }
    let min = totals.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = totals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    println!(
        "OUTER_LIST_PROBE min={min:.1} max={max:.1} delta={:.1}",
        max - min
    );
    assert!(
        (max - min) < 2.0,
        "list content total jittered while scrolling: min={min} max={max} totals={totals:?}"
    );
}
