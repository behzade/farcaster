use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, point, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{ButtonTone, dropdown_button, preserve_pointer_focus};

struct Controls {
    owner: FocusHandle,
    toggled: bool,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.owner)
            .capture_key_down(|event, window, cx| {
                crate::app::ui::focus::traverse_tab(event, None, window, cx);
            })
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("toggle")
                    .tab_index(0)
                    .w(px(100.0))
                    .h(px(40.0))
                    .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggled = !this.toggled;
                        cx.notify();
                    }))
                    .child("Toggle"),
            )
            .child(
                dropdown_button("menu", "Menu", ButtonTone::Neutral, true)
                    .w(px(100.0))
                    .h(px(40.0))
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Choose").on_click(|_, _, _| {}))
                    }),
            )
    }
}

#[gpui::test]
fn pointer_controls_and_dropdowns_restore_the_keyboard_owner(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        Controls {
            owner,
            toggled: false,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_click(point(px(20.0), px(20.0)), Default::default());
    cx.update(|window, cx| {
        assert!(view.read(cx).toggled);
        assert!(view.read(cx).owner.is_focused(window));
    });

    for keys in ["escape", "down enter"] {
        cx.simulate_click(point(px(20.0), px(60.0)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(!view.read(cx).owner.is_focused(window)));
        cx.simulate_keystrokes(keys);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(view.read(cx).owner.is_focused(window)));
    }
    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| assert!(!view.read(cx).owner.is_focused(window)));
}

struct KeyboardHarness {
    normal: FocusHandle,
    composer: gpui::Entity<gpui_component::input::TextareaState>,
    title: gpui::Entity<gpui_component::input::InputState>,
    dialogs: Vec<(FocusHandle, Option<FocusHandle>)>,
    queued: usize,
}

impl KeyboardHarness {
    fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        self.dialogs.push((focus.clone(), window.focused(cx)));
        focus.focus(window, cx);
        cx.notify();
    }

    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((closing, target)) = self.dialogs.pop() {
            crate::app::ui::focus::restore(
                target,
                &closing,
                &self.normal,
                self.normal.clone(),
                window,
                cx,
            );
            cx.notify();
        }
    }
}

impl Render for KeyboardHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_component::input::{Input, Textarea};
        let entity = cx.entity().downgrade();
        let mut root = div()
            .size_full()
            .track_focus(&self.normal)
            .capture_key_down(cx.listener(|this, event, window, cx| {
                if this.dialogs.is_empty() {
                    crate::app::ui::focus::traverse_tab(event, None, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::app::SubmitFollowUp, _, _| this.queued += 1))
            .on_action(
                cx.listener(|this, _: &crate::app::DismissSurface, window, cx| {
                    this.close_dialog(window, cx)
                }),
            )
            .child(
                div()
                    .key_context("FarcasterComposer")
                    .child(Textarea::new(&self.composer)),
            )
            .child(
                div()
                    .debug_selector(|| "title-row".into())
                    .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
                    .child(Input::new(&self.title)),
            )
            .child(
                dropdown_button("open-modal-menu", "Menu", ButtonTone::Neutral, true)
                    .debug_selector(|| "modal-menu-trigger".into())
                    .dropdown_menu(move |menu, _, _| {
                        let entity = entity.clone();
                        menu.item(PopupMenuItem::new("Open dialog").on_click(
                            move |_, window, cx| {
                                let _ = entity.update(cx, |this, cx| this.open_dialog(window, cx));
                            },
                        ))
                    }),
            );
        for (index, (focus, _)) in self.dialogs.iter().enumerate() {
            let dismiss = cx.entity().downgrade();
            root = root.child(super::modal(
                if index == 0 {
                    "first-dialog"
                } else {
                    "nested-dialog"
                },
                "Dialog",
                focus,
                crate::app::OVERLAY_KEY_CONTEXT,
                move |window, cx| {
                    let _ = dismiss.update(cx, |this, cx| this.close_dialog(window, cx));
                },
                |surface| {
                    surface
                        .child(super::button(
                            "first",
                            "First",
                            ButtonTone::Neutral,
                            true,
                            |_, _| {},
                        ))
                        .child(super::button(
                            "second",
                            "Second",
                            ButtonTone::Neutral,
                            true,
                            |_, _| {},
                        ))
                },
            ));
        }
        root
    }
}

fn keyboard_harness(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<KeyboardHarness>, &mut gpui::VisualTestContext) {
    use gpui::AppContext as _;
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys(crate::app::ui::keybindings::bindings());
    });
    cx.add_window_view(|window, cx| {
        let composer = cx
            .new(|cx| gpui_component::input::TextareaState::new(window, cx).submit_on_enter(true));
        let title = cx.new(|cx| gpui_component::input::InputState::new(window, cx));
        let normal = cx.focus_handle();
        normal.focus(window, cx);
        KeyboardHarness {
            normal,
            composer,
            title,
            dialogs: Vec::new(),
            queued: 0,
        }
    })
}

#[gpui::test]
fn modal_tab_cycles_and_nested_escape_restores_in_order(cx: &mut gpui::TestAppContext) {
    let (view, cx) = keyboard_harness(cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_dialog(window, cx));
        window.draw(cx).clear(cx);
    });
    for key in ["tab", "tab", "tab", "shift-tab", "shift-tab", "shift-tab"] {
        cx.simulate_keystrokes(key);
        cx.update(|window, cx| {
            assert!(view.read(cx).dialogs[0].0.contains_focused(window, cx));
        });
    }
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_dialog(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("tab shift-tab tab");
    cx.update(|window, cx| assert!(view.read(cx).dialogs[1].0.contains_focused(window, cx)));
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(view.read(cx).dialogs.len(), 1);
        assert!(view.read(cx).dialogs[0].0.contains_focused(window, cx));
    });
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(view.read(cx).dialogs.is_empty());
        assert!(view.read(cx).normal.is_focused(window));
    });
}

#[gpui::test]
fn composer_tab_keeps_its_action_and_title_accepts_pointer_focus(cx: &mut gpui::TestAppContext) {
    use gpui::Focusable as _;
    let (view, cx) = keyboard_harness(cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        view.read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    });
    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| {
        assert_eq!(view.read(cx).queued, 1);
        assert!(
            view.read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    cx.simulate_keystrokes("shift-tab");
    cx.update(|_, cx| assert_eq!(view.read(cx).queued, 1));
    let bounds = cx.debug_bounds("title-row").expect("title row rendered");
    cx.simulate_click(bounds.center(), Default::default());
    cx.update(|window, cx| {
        assert!(
            view.read(cx)
                .title
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
    cx.simulate_input("Renamed session");
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).title.read(cx).value().as_ref(),
            "Renamed session"
        )
    });
}

#[gpui::test]
fn menu_opening_dialog_does_not_steal_the_new_dialog_focus(cx: &mut gpui::TestAppContext) {
    let (view, cx) = keyboard_harness(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx
        .debug_bounds("modal-menu-trigger")
        .expect("menu trigger rendered");
    cx.simulate_click(bounds.center(), Default::default());
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(view.read(cx).dialogs.len(), 1);
        assert!(view.read(cx).dialogs[0].0.is_focused(window));
    });
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(view.read(cx).normal.is_focused(window));
    });
}
