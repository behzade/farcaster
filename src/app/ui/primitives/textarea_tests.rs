use super::*;
use gpui::{Focusable as _, IntoElement, Render};

struct Form {
    input: Entity<TextareaState>,
    submissions: Vec<String>,
    _subscription: Subscription,
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(submit_textarea(Textarea::new(&self.input)))
    }
}

#[gpui::test]
fn enter_submits_and_shift_enter_inserts_newline(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let (input, subscription) = create_submit_textarea(
            window,
            cx,
            |input| input.auto_grow(2, 6),
            |this: &mut Form, _, cx| {
                this.submissions
                    .push(this.input.read(cx).value().to_string());
            },
        );
        input.read(cx).focus_handle(cx).focus(window, cx);
        Form {
            input,
            submissions: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_input("Subject");
    cx.simulate_keystrokes("shift-enter");
    cx.update(|_, cx| {
        let form = view.read(cx);
        assert!(form.submissions.is_empty());
        assert_eq!(form.input.read(cx).value().as_ref(), "Subject\n");
    });
    cx.simulate_input("Body");
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let form = view.read(cx);
        assert_eq!(form.submissions, ["Subject\nBody"]);
        assert_eq!(form.input.read(cx).value().as_ref(), "Subject\nBody");
    });
}
