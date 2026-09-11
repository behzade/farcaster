use std::{collections::HashSet, path::Path};

use gpui::{AppContext as _, Context, Entity, Focusable as _, Subscription, Window};
use gpui_component::list::{ListEvent, ListState};

use crate::{
    app::{
        FarcasterApp,
        composer::sessions::session_target,
        ui::{
            assets::AppIcon,
            primitives::{PickerDelegate, PickerRow},
        },
    },
    sessions::{SessionSummary, SessionTarget},
};

#[derive(Clone)]
pub(in crate::app) struct CodeDestination {
    pub target: String,
    pub session: Option<SessionTarget>,
    pub label: String,
    pub harness: String,
}

pub(in crate::app) struct DestinationPicker {
    pub list: Entity<ListState<PickerDelegate>>,
    _subscription: Subscription,
}

pub(super) fn choices(
    project: &Path,
    current: CodeDestination,
    sessions: &[SessionSummary],
) -> Vec<CodeDestination> {
    let mut seen = HashSet::from([current.target.clone()]);
    if let Some(session) = &current.session {
        seen.insert(session_target(&session.path));
    }
    let mut candidates = sessions
        .iter()
        .filter(|session| !session.archived && session.project == project)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|session| std::cmp::Reverse(session.modified));
    let mut choices = vec![current];
    for session in candidates {
        let target = session_target(&session.path);
        if seen.insert(target.clone()) {
            choices.push(CodeDestination {
                target,
                session: Some(session.target()),
                label: session.title.clone(),
                harness: session.harness.clone(),
            });
        }
    }
    choices
}

// None is the "New task" entry before the existing chats in the picker.
pub(super) fn cycle_destination(
    current: Option<usize>,
    count: usize,
    forward: bool,
) -> Option<usize> {
    if forward {
        let next = current.map_or(0, |index| index + 1);
        (next < count).then_some(next)
    } else {
        current.unwrap_or(count).checked_sub(1)
    }
}

impl FarcasterApp {
    pub(in crate::app) fn cycle_code_destination(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(dialog) = self.send_to_chat.as_mut() else {
            return;
        };
        // The open picker's list owns navigation until it is confirmed or cancelled.
        if dialog.picker.is_some() {
            return;
        }
        dialog.destination =
            cycle_destination(dialog.destination, dialog.destinations.len(), forward);
        cx.notify();
    }

    pub(in crate::app) fn choose_code_destination(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.send_to_chat.as_ref() else {
            return;
        };
        let mut rows = vec![PickerRow::new(
            "new",
            AppIcon::ChatCircle,
            "New task",
            None,
            None,
            "",
        )];
        rows.extend(
            dialog
                .destinations
                .iter()
                .enumerate()
                .map(|(index, destination)| {
                    PickerRow::new(
                        index.to_string(),
                        AppIcon::ChatCircle,
                        destination.label.clone(),
                        Some(crate::agents::backend_display_name(&destination.harness).to_owned()),
                        None,
                        "",
                    )
                }),
        );
        let (delegate, handles) = PickerDelegate::new(rows);
        let selected = dialog.destination.map_or(0, |index| index + 1);
        let list = cx.new(|cx| ListState::new(delegate, window, cx).searchable(true));
        let subscription =
            cx.subscribe_in(&list, window, move |_, _, event, window, cx| match event {
                ListEvent::Confirm(_) => {
                    let id = handles.confirmed_id.borrow_mut().take();
                    cx.defer_in(window, move |this, window, cx| {
                        if let (Some(dialog), Some(id)) = (this.send_to_chat.as_mut(), id) {
                            if id == "new" {
                                dialog.destination = None;
                            } else if let Ok(index) = id.parse::<usize>()
                                && index < dialog.destinations.len()
                            {
                                dialog.destination = Some(index);
                            }
                        }
                        this.close_code_destination_picker(window, cx);
                    });
                    cx.stop_propagation();
                }
                ListEvent::Cancel => {
                    cx.defer_in(window, |this, window, cx| {
                        this.close_code_destination_picker(window, cx)
                    });
                    cx.stop_propagation();
                }
                ListEvent::Select(_) => {}
            });
        list.update(cx, |list, cx| {
            list.set_selected_index(
                Some(gpui_component::IndexPath {
                    row: selected,
                    ..Default::default()
                }),
                window,
                cx,
            );
            list.focus(window, cx);
        });
        self.send_to_chat.as_mut().expect("open capture").picker = Some(DestinationPicker {
            list,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_code_destination_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.send_to_chat.as_mut() {
            dialog.picker = None;
            dialog.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }
}
