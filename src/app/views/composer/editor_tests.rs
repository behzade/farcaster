use super::*;
use gpui::{AppContext as _, Context, EntityInputHandler as _, Focusable as _, Render};
use gpui_component::input::{InputEvent, TextDecoration};

struct InputHarness {
    input: Entity<EditorState>,
    presentation: ComposerPresentation,
    decorations: TextDecorationCollection,
    submissions: usize,
    _subscription: gpui::Subscription,
}

impl Render for InputHarness {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        composer_editor(&self.input, composer_line_height() * 8)
    }
}

#[gpui::test]
fn composer_remains_plain_text_and_restores_highlights_after_draft_replacement(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let input = cx.new(|cx| crate::app::bootstrap::composer_input(window, cx));
        input.read(cx).focus_handle(cx).focus(window, cx);
        let decorations = input.update(cx, |input, cx| {
            input.create_decorations_collection(Vec::new(), cx)
        });
        let subscription = cx.subscribe(&input, |this: &mut InputHarness, _, event, _| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.submissions += 1;
            }
        });
        InputHarness {
            input,
            presentation: ComposerPresentation::default(),
            decorations,
            submissions: 0,
            _subscription: subscription,
        }
    });
    cx.simulate_input("  /skill فارسی");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("(");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert_eq!(view.input.read(cx).value().as_ref(), "  /skill فارسی\n(");
            assert_eq!(view.submissions, 0);
            let decorations = vec![TextDecoration::new(
                2..8,
                gpui::HighlightStyle {
                    color: Some(theme().colors.skill.into()),
                    ..Default::default()
                },
            )];
            let value = view.input.read(cx).value();
            view.presentation
                .update_highlights(&view.decorations, &value, decorations.clone(), cx);
            assert_eq!(view.decorations.get_ranges(cx), [2..8]);
            // Switching drafts clears annotations even when the text stays the same.
            view.input
                .update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            view.presentation
                .update_highlights(&view.decorations, &value, decorations.clone(), cx);
            assert_eq!(view.decorations.get_ranges(cx), [2..8]);
            // A second render scope must replace this same collection.
            ComposerPresentation::default().update_highlights(
                &view.decorations,
                &value,
                Vec::new(),
                cx,
            );
            assert!(view.decorations.get_ranges(cx).is_empty());
            view.presentation
                .update_highlights(&view.decorations, &value, decorations, cx);
            assert_eq!(view.decorations.get_ranges(cx), [2..8]);
            view.input.update(cx, |input, cx| {
                input.set_selected_range(2..8, cx);
                input.replace_text_in_range(None, "سلام", window, cx);
            });
            assert_eq!(view.input.read(cx).value().as_ref(), "  سلام فارسی\n(");
        })
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).input.read(cx).value().as_ref(),
            "  /skill فارسی\n("
        )
    });
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| assert_eq!(view.read(cx).submissions, 1));
}

#[gpui::test]
fn composer_grows_with_wrapping_and_caps_at_eight_rows(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let line_height = composer_line_height();
        assert_eq!(
            composer_height(&"".into(), Some(px(100.)), window),
            line_height
        );
        assert_eq!(
            composer_height(&"one\ntwo\nthree".into(), Some(px(100.)), window),
            line_height * 3
        );
        let text = "word ".repeat(20).into();
        assert!(
            composer_height(&text, Some(px(100.)), window)
                > composer_height(&text, Some(px(800.)), window)
        );
        assert_eq!(
            composer_height(&"line\n".repeat(100).into(), Some(px(100.)), window),
            line_height * 8
        );
    });
}

#[gpui::test]
fn composer_clicks_focus_blank_space_and_keep_visible_text_in_place(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::composer_clicks_focus_blank_space_and_keep_visible_text_in_place"
        ),
        cx,
        |cx, app, _, _| {
            cx.simulate_resize(gpui::size(px(900.), px(700.)));
            let draw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| window.draw(cx).clear(cx));
            };
            let input = cx.update(|window, cx| {
                let input = app.read(cx).composer.input.clone();
                input.update(cx, |input, cx| {
                    input.set_value("one\ntwo\nthree", window, cx);
                    input.set_selected_range(0..0, cx);
                });
                input
            });
            draw(cx);
            draw(cx);
            cx.update(|window, cx| {
                let input = input.read(cx);
                // Match the old textarea's text_sm and 1.25rem styles.
                assert_eq!(
                    input.line_height(),
                    Some((theme().type_scale.body * 1.25).round())
                );
                let text = "one";
                let expected = window.text_system().shape_line(
                    text.into(),
                    theme().type_scale.body * 0.875,
                    &[gpui::TextRun {
                        len: text.len(),
                        font: gpui::font(UI_FONT_FAMILY),
                        color: theme().colors.text.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                assert!(
                    (input.range_to_bounds(&(0..3)).unwrap().size.width - expected.width).abs()
                        < px(0.01)
                );
            });
            for draft in [true, false] {
                if !draft {
                    app.update(cx, |app, cx| {
                        app.sessions.selected_draft = None;
                        app.notify_composer(cx);
                        cx.notify();
                    });
                    draw(cx);
                }
                let bounds = cx.update(|_, cx| input.read(cx).input_bounds());
                for position in [
                    point(bounds.right() - px(20.), bounds.top() + px(10.)),
                    point(bounds.left() - px(10.), bounds.top() + px(10.)),
                ] {
                    cx.update(|window, cx| window.blur(cx));
                    draw(cx);
                    cx.simulate_click(position, Default::default());
                    cx.update(|window, cx| {
                        assert!(
                            input.read(cx).focus_handle(cx).is_focused(window),
                            "blank click at {position:?}, draft={draft} must focus composer"
                        );
                    });
                }
            }
            let bounds = cx.update(|_, cx| input.read(cx).input_bounds());
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: bounds.center(),
                delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1_000.))),
                ..Default::default()
            });
            draw(cx);
            cx.update(|_, cx| {
                assert_eq!(
                    input.read(cx).scroll_offset().y,
                    px(0.),
                    "short text must not scroll into empty editor rows"
                );
            });
            cx.update(|window, cx| {
                input.update(cx, |input, cx| {
                    input.set_value("word\n".repeat(12), window, cx);
                    input.set_selected_range(0..0, cx);
                    input.set_scroll_offset(point(px(0.), px(0.)), cx);
                });
            });
            draw(cx);
            draw(cx);
            let (bounds, line_height) = cx.update(|_, cx| {
                let input = input.read(cx);
                assert_eq!(input.scroll_offset().y, px(0.));
                assert_eq!(
                    input.input_bounds().size.height,
                    input.line_height().unwrap() * 8.
                );
                (input.input_bounds(), input.line_height().unwrap())
            });
            cx.simulate_click(
                bounds.origin + point(px(8.), line_height * 7.5),
                Default::default(),
            );
            draw(cx);
            cx.update(|window, cx| {
                let input = input.read(cx);
                assert_eq!(input.cursor_position().line, 7);
                assert!(input.focus_handle(cx).is_focused(window));
                assert_eq!(
                    input.scroll_offset().y,
                    px(0.),
                    "clicking visible text must not scroll it"
                );
            });
        },
    );
}
