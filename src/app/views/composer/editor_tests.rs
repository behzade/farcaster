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
        composer_editor(&self.input, theme().type_scale.line_composer * 8)
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
        let line_height = theme().type_scale.line_composer;
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
