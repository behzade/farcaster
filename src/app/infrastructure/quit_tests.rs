use super::*;
use gpui::{InteractiveElement as _, Render, TestAppContext, div};

struct ActiveWorkView {
    focus: gpui::FocusHandle,
}

impl ActiveWorkView {
    fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _answer = window.prompt(
            PromptLevel::Warning,
            "Exit Farcaster?",
            Some("Agents are still active."),
            &[PromptButton::cancel("Cancel"), PromptButton::ok("Exit")],
            cx,
        );
    }
}

impl Render for ActiveWorkView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .track_focus(&self.focus)
            .key_context(crate::app::APP_INPUT_CONTEXT)
    }
}

fn assert_quit_prompts(
    cx: &mut TestAppContext,
    request: impl FnOnce(&mut gpui::VisualTestContext),
) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        ActiveWorkView { focus }
    });
    cx.update(|window, cx| {
        install(
            Rc::new(RefCell::new(Some(view.downgrade()))),
            ActiveWorkView::request_quit,
            cx,
        );
        install_window(window, cx);
        cx.bind_keys(crate::app::ui::keybindings::bindings());
        #[cfg(target_os = "macos")]
        crate::app::infrastructure::menus::install(cx);
        window.draw(cx).clear(cx);
    });
    assert!(!cx.has_pending_prompt());

    request(cx);
    assert!(
        cx.has_pending_prompt(),
        "quit request bypassed the active-work quit guard"
    );
    cx.simulate_prompt_answer("Cancel");
}

#[gpui::test]
fn quit_shortcut_prompts_for_active_work(cx: &mut TestAppContext) {
    assert_quit_prompts(cx, |cx| {
        cx.simulate_keystrokes(&crate::app::ui::keybindings::application_key("q"));
    });
}

#[cfg(target_os = "linux")]
#[gpui::test]
fn super_quit_shortcut_prompts_for_active_work(cx: &mut TestAppContext) {
    assert_quit_prompts(cx, |cx| cx.simulate_keystrokes("super-q"));
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn close_window_shortcut_prompts_for_active_work(cx: &mut TestAppContext) {
    assert_quit_prompts(cx, |cx| cx.simulate_keystrokes("cmd-shift-w"));
}

#[gpui::test]
fn native_window_close_prompts_for_active_work(cx: &mut TestAppContext) {
    assert_quit_prompts(cx, |cx| {
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(cx.active_window(), Some(window.window_handle()));
        });
        assert!(!cx.simulate_close(), "window closed before confirmation");
    });
}
