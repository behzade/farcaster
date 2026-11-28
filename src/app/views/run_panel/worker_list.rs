use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, FocusHandle, IntoElement as _, ParentElement as _, Pixels, Point,
    Styled as _, UniformListDecoration, UniformListScrollHandle, Window, div, point, px,
};

#[derive(Clone)]
pub(super) struct WorkerRow {
    pub activity: crate::agent_activity::AgentActivity,
    pub depth: usize,
    pub session: crate::sessions::SessionSummary,
    pub session_key: crate::sessions::SessionKey,
    pub focus: Option<FocusHandle>,
}

pub(in crate::app::views) struct PreparedWorkers {
    pub(in crate::app::views) root: Option<crate::sessions::SessionKey>,
    pub(super) rows: Vec<Rc<WorkerRow>>,
    pub(super) older_by_key: std::collections::HashMap<crate::sessions::SessionKey, usize>,
    pub(super) visible: Rc<Vec<Rc<WorkerRow>>>,
    pub(super) keys: Rc<Vec<(String, FocusHandle)>>,
}

#[cfg(test)]
thread_local! {
    pub(super) static PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(in crate::app::views) type WorkerKeys = Rc<Vec<(String, FocusHandle)>>;

pub(super) type RenderRow = Rc<dyn Fn(usize, &mut App) -> AnyElement>;

pub(super) struct FocusedRow {
    pub handles: Rc<Vec<(String, FocusHandle)>>,
    pub render: RenderRow,
}

impl UniformListDecoration for FocusedRow {
    fn compute(
        &self,
        visible: Range<usize>,
        _: Bounds<Pixels>,
        _: Point<Pixels>,
        height: Pixels,
        _: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let focused = window.focused(cx).and_then(|focused| {
            self.handles
                .iter()
                .position(|(_, handle)| handle == &focused)
        });
        let Some(index) = focused.filter(|index| !visible.contains(index)) else {
            return div().into_any_element();
        };
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top(height * index)
                    .w_full()
                    .h(height)
                    .child((self.render)(index, cx)),
            )
            .into_any_element()
    }
}

pub(super) fn reconcile(
    scroll: &UniformListScrollHandle,
    current: &RefCell<WorkerKeys>,
    next: &WorkerKeys,
    height: Pixels,
) {
    let mut current = current.borrow_mut();
    if Rc::ptr_eq(&current, next) {
        return;
    }
    if current.as_slice() == next.as_slice() {
        *current = next.clone();
        return;
    }
    let mut state = scroll.0.borrow_mut();
    let offset = state.base_handle.offset();
    let top = (-offset.y / height).floor().max(0.0) as usize;
    let next_top = current
        .get(top)
        .and_then(|(key, _)| next.iter().position(|(next, _)| next == key))
        .unwrap_or_else(|| top.min(next.len().saturating_sub(1)));
    let within_row = -offset.y - height * top;
    let y = if next.is_empty() {
        px(0.0)
    } else {
        -(height * next_top + within_row)
    };
    state.base_handle.set_offset(point(offset.x, y));
    state.deferred_scroll_to_item = state.deferred_scroll_to_item.and_then(|mut pending| {
        if current.is_empty() {
            return Some(pending);
        }
        let (key, _) = current.get(pending.item_index)?;
        pending.item_index = next.iter().position(|(next, _)| next == key)?;
        Some(pending)
    });
    *current = next.clone();
}
