use super::*;
use crate::{
    app::composer::queue::prepare,
    conversation::{PendingReceipt, QueueState},
    protocol::PromptMode,
};
use gpui::{Context, FocusHandle, Render, px};
use std::{cell::RefCell, rc::Rc};

struct QueueHarness {
    queue: QueueState,
    receipts: Vec<PendingReceipt>,
    focus: FocusHandle,
    width: f32,
    pressed: Rc<RefCell<Vec<String>>>,
}

impl QueueHarness {
    fn new(queue: QueueState, width: f32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            queue,
            receipts: Vec::new(),
            focus,
            width,
            pressed: Rc::default(),
        }
    }
}

impl Render for QueueHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pressed = self.pressed.clone();
        div()
            .track_focus(&self.focus)
            .capture_key_down(|event, window, cx| {
                crate::app::ui::focus::traverse_tab(event, None, window, cx);
            })
            .w(px(self.width))
            .children(render(
                prepare(
                    &self.queue,
                    &self
                        .receipts
                        .iter()
                        .map(PendingReceipt::as_ref)
                        .collect::<Vec<_>>(),
                    "test",
                    Some(std::path::Path::new("/test/session")),
                    true,
                    false,
                ),
                Rc::new(move |action, _| pressed.borrow_mut().push(action.id.clone())),
            ))
    }
}

#[gpui::test]
fn queue_groups_keep_expansion_warnings_and_cancel_permissions(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let mut view = QueueHarness::new(
            QueueState {
                steering: vec!["check this first".into(), "uncertain message".into()],
                steering_ids: vec!["steer".into(), "unknown".into()],
                follow_up: vec!["then test\nincluding the empty state".into()],
                follow_up_ids: vec!["later".into()],
                cancellable_ids: vec!["steer".into()],
                ..Default::default()
            },
            220.0,
            window,
            cx,
        );
        view.receipts.push(PendingReceipt {
            id: "unknown".into(),
            mode: Some(PromptMode::Steer),
            text: "uncertain message".into(),
            images: Default::default(),
            unknown: true,
        });
        view
    });
    for width in [220.0, 480.0] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.width = width;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        let steer = "queue-row:test:Steer:steer";
        let follow_up = "queue-row:test:FollowUp:later";
        assert!(cx.debug_bounds("queue-group-Steer").is_some());
        assert!(cx.debug_bounds("queue-group-FollowUp").is_some());
        assert!(cx.debug_bounds(steer).is_some());
        let preview = cx.debug_bounds(follow_up).expect("follow-up row");
        assert!(preview.right() <= px(width));
        assert!(preview.size.height <= theme().layout.status_row_height);
        assert!(cx.debug_bounds("queue-row-notice").is_some());
        assert!(cx.debug_bounds("cancel-queue-steer").is_some());
        assert!(cx.debug_bounds("cancel-queue-unknown").is_none());
        assert!(cx.debug_bounds("cancel-queue-later").is_none());

        let preview = cx
            .debug_bounds("queue-row:test:FollowUp:later-preview")
            .expect("follow-up preview");
        cx.simulate_click(preview.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let body = cx
            .debug_bounds("queue-message-body")
            .expect("expanded body");
        assert!(body.right() <= px(width));
        cx.simulate_click(preview.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.queue.steering.clear();
            view.queue.steering_ids.clear();
            view.receipts.clear();
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("queue-row:test:FollowUp:later").is_some());
    assert!(cx.debug_bounds("queue-group-Steer").is_none());
    assert!(cx.debug_bounds("queue-group-FollowUp").is_some());
    assert!(cx.debug_bounds("queue-row-notice").is_none());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.queue = QueueState::default();
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("composer-queue-tray").is_none());
}

#[gpui::test]
fn queue_expands_only_hidden_text_and_updates_after_resize(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        QueueHarness::new(
            QueueState {
                steering: vec!["hi".into()],
                steering_ids: vec!["message".into()],
                ..Default::default()
            },
            200.0,
            window,
            cx,
        )
    });
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    let click = |cx: &mut gpui::VisualTestContext| {
        let preview = cx
            .debug_bounds("queue-row:test:Steer:message-preview")
            .unwrap();
        cx.simulate_click(preview.center(), Default::default());
        draw(cx);
    };
    draw(cx);
    click(cx);
    assert!(cx.debug_bounds("queue-message-body").is_none());
    cx.simulate_keystrokes("tab enter space");
    draw(cx);
    assert!(cx.debug_bounds("queue-message-body").is_none());

    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.queue.steering[0] =
                "a long single line that fits only when the row is wide".into();
            cx.notify();
        })
    });
    draw(cx);
    click(cx);
    assert!(
        cx.debug_bounds("queue-message-body").is_some(),
        "clipped single line must expand"
    );
    click(cx);
    assert!(cx.debug_bounds("queue-message-body").is_none());
    cx.update(|window, cx| view.read(cx).focus.clone().focus(window, cx));
    cx.simulate_keystrokes("tab enter");
    draw(cx);
    assert!(
        cx.debug_bounds("queue-message-body").is_some(),
        "keyboard must expand clipped text"
    );
    cx.simulate_keystrokes("space");
    draw(cx);
    assert!(cx.debug_bounds("queue-message-body").is_none());
    click(cx);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.width = 900.0;
            cx.notify();
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("queue-message-body").is_none(),
        "widening removes redundant expansion"
    );
    click(cx);
    assert!(cx.debug_bounds("queue-message-body").is_none());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.width = 200.0;
            cx.notify();
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("queue-message-body").is_none(),
        "resizing must not reopen the body"
    );
    click(cx);
    assert!(cx.debug_bounds("queue-message-body").is_some());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.queue.steering[0] = "short again".into();
            cx.notify();
        })
    });
    draw(cx);
    click(cx);
    assert!(
        cx.debug_bounds("queue-message-body").is_none(),
        "same ID with new short text must reset expansion"
    );
}

#[gpui::test]
fn queue_details_keep_actions_reachable_when_preview_matches_body(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        QueueHarness::new(
            QueueState {
                saved: vec![crate::conversation::SavedPrompt {
                    id: 7,
                    target: "saved-target".into(),
                    text: "hi".into(),
                    image_count: 1,
                    sendable: true,
                }],
                ..Default::default()
            },
            480.0,
            window,
            cx,
        )
    });
    let pressed = cx.update(|_, cx| view.read(cx).pressed.clone());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("queue-row-notice").is_some());
    let preview = cx.debug_bounds("queue-recovered:test:7-preview").unwrap();
    cx.simulate_click(preview.center(), Default::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("queue-message-body").is_some());
    let send = cx.debug_bounds("send-saved-7").expect("send button");
    let remove = cx.debug_bounds("remove-saved-7").expect("remove button");
    cx.simulate_click(send.center(), Default::default());
    assert_eq!(*pressed.borrow(), ["send-saved-7"]);
    cx.simulate_click(remove.center(), Default::default());
    assert_eq!(*pressed.borrow(), ["send-saved-7", "remove-saved-7"]);
    pressed.borrow_mut().clear();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.queue.saved[0].sendable = false;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.simulate_click(send.center(), Default::default());
    assert!(
        pressed.borrow().is_empty(),
        "disabled send must not dispatch"
    );
    cx.simulate_click(remove.center(), Default::default());
    assert_eq!(*pressed.borrow(), ["remove-saved-7"]);
}
