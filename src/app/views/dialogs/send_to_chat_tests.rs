use super::*;
use crate::app::ui::{
    assets::AppIcon,
    primitives::{PickerDelegate, PickerRow},
};
use gpui::{AppContext as _, Context, Entity, Focusable as _, Render, Window};
use gpui_component::{IndexPath, input::TextareaState, list::ListState};

struct DestinationForm {
    input: Entity<TextareaState>,
    picker: Entity<ListState<PickerDelegate>>,
    show_picker: bool,
    moves: Vec<bool>,
}

impl Render for DestinationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .size_full()
            .key_context("FarcasterSendToChat")
            .on_action(cx.listener(|this, _: &NextCodeDestination, _, _| this.moves.push(true)))
            .on_action(
                cx.listener(|this, _: &PreviousCodeDestination, _, _| this.moves.push(false)),
            )
            .child(if self.show_picker {
                List::new(&self.picker).into_any_element()
            } else {
                submit_textarea(Textarea::new(&self.input)).into_any_element()
            })
    }
}

#[gpui::test]
fn destination_keys_bypass_text_selection_and_still_navigate_the_picker(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys(crate::app::ui::keybindings::bindings());
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .submit_on_enter(true)
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        let (delegate, _) = PickerDelegate::new(
            ["New task", "Current chat", "Recent chat"]
                .into_iter()
                .map(|label| PickerRow::new(label, AppIcon::ChatCircle, label, None, None, ""))
                .collect(),
        );
        let picker = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));
        picker.update(cx, |picker, cx| {
            picker.set_selected_index(Some(IndexPath::default()), window, cx);
        });
        DestinationForm {
            input,
            picker,
            show_picker: false,
            moves: Vec::new(),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_input("First\nSecond");
    let (cursor, selection) = cx.update(|_, cx| {
        let input = view.read(cx).input.read(cx);
        (input.cursor(), input.selected_range())
    });
    cx.simulate_keystrokes("ctrl-p ctrl-n");
    cx.update(|window, cx| {
        let form = view.read(cx);
        assert_eq!(form.moves, [false, true]);
        let input = form.input.read(cx);
        assert_eq!(input.value().as_ref(), "First\nSecond");
        assert_eq!(input.cursor(), cursor);
        assert_eq!(input.selected_range(), selection);
        assert!(input.focus_handle(cx).is_focused(window));
    });
    cx.simulate_keystrokes("shift-up");
    cx.update(|_, cx| {
        let form = view.read(cx);
        assert!(!form.input.read(cx).selected_range().is_empty());
        assert_eq!(form.moves, [false, true]);
    });
    cx.update(|window, cx| {
        view.update(cx, |form, cx| {
            form.show_picker = true;
            form.picker
                .update(cx, |picker, cx| picker.focus(window, cx));
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    for (key, row) in [("ctrl-n", 1), ("ctrl-n", 2), ("ctrl-p", 1)] {
        cx.simulate_keystrokes(key);
        cx.update(|window, cx| {
            let form = view.read(cx);
            let picker = form.picker.read(cx);
            assert_eq!(picker.selected_index().map(|index| index.row), Some(row));
            assert!(picker.focus_handle(cx).is_focused(window));
            assert_eq!(form.moves, [false, true]);
        });
    }
}
