use super::*;
use gpui::{
    AppContext as _, Context, FocusHandle, Focusable as _, InteractiveElement as _,
    ParentElement as _, Render, VisualTestContext, div, px,
};
use gpui_component::{Root, WindowExt as _, input::AnyInputState};

struct Fields {
    inputs: [Entity<InputState>; 2],
    other: FocusHandle,
    disabled: bool,
}

impl Render for Fields {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("other")
                    .debug_selector(|| "other".into())
                    .track_focus(&self.other)
                    .w(px(200.))
                    .h(px(30.))
                    .child("Other"),
            )
            .children(self.inputs.iter().enumerate().map(|(index, state)| {
                div()
                    .debug_selector(move || format!("field-{index}"))
                    .w(px(200.))
                    .child(
                        Input::new(state)
                            .appearance(index == 0)
                            .disabled(self.disabled),
                    )
            }))
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn frame_clicks_focus_the_editor_for_all_inputs(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let mut fields = None;
    let window = cx.add_window(|window, cx| {
        let inputs = std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx)));
        let view = cx.new(|cx| Fields {
            inputs,
            other: cx.focus_handle(),
            disabled: false,
        });
        fields = Some(view.clone());
        Root::new(view, window, cx)
    });
    let fields = fields.unwrap();
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    draw(cx);
    for index in 0..2 {
        for edge in [0, 1, 2] {
            let other = cx.debug_bounds("other").unwrap();
            cx.simulate_click(other.center(), Default::default());
            draw(cx);
            let input = cx.update(|_, cx| fields.read(cx).inputs[index].clone());
            let bounds = cx.debug_bounds(["field-0", "field-1"][index]).unwrap();
            let position = match edge {
                0 => gpui::point(bounds.left() + px(2.), bounds.center().y),
                1 => bounds.center(),
                _ => gpui::point(bounds.right() - px(2.), bounds.center().y),
            };
            cx.simulate_event(MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            draw(cx);
            cx.simulate_event(MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            draw(cx);
            cx.update(|window, cx| {
                assert!(
                    input.read(cx).focus_handle(cx).is_focused(window),
                    "input={index} edge={edge}"
                );
                assert_eq!(
                    window.focused_input(cx),
                    Some(AnyInputState::Input(input.clone()))
                );
            });
            cx.simulate_input("x");
            cx.update(|_, cx| assert_eq!(input.read(cx).value().len(), edge + 1));
        }
    }
    cx.update(|_, cx| {
        fields.update(cx, |fields, cx| {
            fields.disabled = true;
            cx.notify();
        })
    });
    draw(cx);
    let other = cx.debug_bounds("other").unwrap();
    cx.simulate_click(other.center(), Default::default());
    draw(cx);
    let bounds = cx.debug_bounds("field-0").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    draw(cx);
    cx.update(|window, cx| assert!(fields.read(cx).other.is_focused(window)));
}
