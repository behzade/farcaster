use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    Pixels, Styled as _, Window, div,
};

use crate::app::ui::theme::theme;

#[derive(Clone, Copy)]
pub(crate) struct ResizeBounds {
    pub height: Pixels,
    pub min_height: Pixels,
    pub max_height: Pixels,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ResizeState {
    height: Option<Pixels>,
    collapsed: bool,
    resize_start: Option<(Pixels, Pixels)>,
}

impl ResizeState {
    pub(crate) fn restored(height: Option<f32>, collapsed: bool) -> Self {
        Self {
            height: height.map(gpui::px),
            collapsed,
            resize_start: None,
        }
    }

    pub(crate) fn stored_height(&self) -> Option<f32> {
        self.height.map(f32::from)
    }

    pub(crate) fn set_height(&mut self, height: Pixels) {
        self.height = Some(height);
    }

    pub(crate) fn height_or(&self, fallback: Pixels) -> Pixels {
        self.height.unwrap_or(fallback)
    }

    pub(crate) fn height(&self, bounds: ResizeBounds) -> Pixels {
        self.height
            .unwrap_or(bounds.height)
            .clamp(bounds.min_height, bounds.max_height)
    }

    pub(crate) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub(crate) fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    pub(crate) fn is_resizing(&self) -> bool {
        self.resize_start.is_some()
    }

    pub(crate) fn begin_resize(&mut self, bounds: ResizeBounds, pointer_y: Pixels) {
        self.resize_start = Some((pointer_y, self.height(bounds)));
    }

    pub(crate) fn update_resize(&mut self, bounds: ResizeBounds, pointer_y: Pixels) -> bool {
        let Some((start_y, start_height)) = self.resize_start else {
            return false;
        };
        let height =
            (start_height + start_y - pointer_y).clamp(bounds.min_height, bounds.max_height);
        let current = self
            .height
            .map(|height| height.clamp(bounds.min_height, bounds.max_height));
        if Some(height) == current {
            return false;
        }
        self.height = Some(height);
        true
    }

    pub(crate) fn finish_resize(&mut self) -> bool {
        self.resize_start.take().is_some()
    }
}

pub(crate) fn resize_handle(
    id: impl Into<ElementId>,
    on_start: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(theme().size(5.0))
        .cursor_row_resize()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            on_start(event, window, cx);
        })
        .into_any_element()
}
