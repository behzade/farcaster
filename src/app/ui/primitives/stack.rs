use gpui::{
    AnyElement, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, Styled as _, div, px,
};

use super::resize::ResizeBounds;
use crate::app::ui::theme::theme;

#[derive(Clone, Copy)]
pub(crate) struct PanelSlot {
    pub floor: Pixels,
    pub preferred: Pixels,
}

impl PanelSlot {
    pub(crate) fn new(floor: Pixels, preferred: Pixels) -> Self {
        Self { floor, preferred }
    }
}

pub(crate) fn panel_space(stack_height: Pixels, surface_floor: Pixels) -> Pixels {
    (stack_height - surface_floor).max(px(0.0))
}

pub(crate) fn panel_heights(sizes: &[Pixels], floors: &[Pixels], budget: Pixels) -> Vec<Pixels> {
    let mut heights = at_least_floors(sizes, floors);
    let mut shortfall = heights.iter().fold(px(0.0), |total, h| total + *h) - budget;
    for index in (0..heights.len()).rev() {
        if shortfall <= px(0.0) {
            break;
        }
        let given = (heights[index] - floors[index]).min(shortfall);
        heights[index] -= given;
        shortfall -= given;
    }
    heights
}

pub(crate) fn panel_max(
    sizes: &[Pixels],
    floors: &[Pixels],
    budget: Pixels,
    index: usize,
) -> Pixels {
    let above = floors[..index].iter().fold(px(0.0), |total, f| total + *f);
    let below = sizes[index + 1..].iter().fold(px(0.0), |total, s| total + *s);
    (budget - above - below).max(floors[index])
}

pub(crate) fn panel_resized(
    sizes: &[Pixels],
    floors: &[Pixels],
    budget: Pixels,
    index: usize,
    size: Pixels,
) -> Vec<Pixels> {
    let mut next = at_least_floors(sizes, floors);
    let taken = size.clamp(floors[index], panel_max(sizes, floors, budget, index)) - next[index];
    next[index] += taken;
    let mut given = taken;
    for above in (0..index).rev() {
        let give = (next[above] - floors[above]).min(given);
        next[above] -= give;
        given -= give;
    }
    next
}

fn at_least_floors(sizes: &[Pixels], floors: &[Pixels]) -> Vec<Pixels> {
    sizes
        .iter()
        .zip(floors)
        .map(|(size, floor)| (*size).max(*floor))
        .collect()
}

pub(crate) fn panel_bounds(room: Option<Pixels>, slot: PanelSlot) -> ResizeBounds {
    let ceiling = room
        .unwrap_or(theme().layout.notice_panel_max)
        .max(slot.floor);
    ResizeBounds {
        height: slot.preferred.clamp(slot.floor, ceiling),
        min_height: slot.floor,
        max_height: ceiling,
        row_height: None,
    }
}

#[derive(IntoElement)]
pub(crate) struct PanelStack {
    id: SharedString,
    children: Vec<AnyElement>,
}

impl PanelStack {
    pub(crate) fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            children: Vec::new(),
        }
    }

    pub(crate) fn surface(mut self, element: impl IntoElement) -> Self {
        self.children.push(element.into_any_element());
        self
    }

    pub(crate) fn child(mut self, element: impl IntoElement) -> Self {
        self.children.push(element.into_any_element());
        self
    }
}

impl RenderOnce for PanelStack {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        let Self { id, children } = self;
        div()
            .id(ElementId::Name(id))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .children(children)
    }
}

#[cfg(test)]
#[path = "stack_tests.rs"]
mod tests;
