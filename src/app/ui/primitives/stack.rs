use gpui::{Pixels, px};

use super::resize::ResizeBounds;

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

pub(crate) fn panel_bounds(room: Pixels, slot: PanelSlot) -> ResizeBounds {
    let ceiling = room.max(slot.floor);
    ResizeBounds {
        height: slot.preferred.clamp(slot.floor, ceiling),
        min_height: slot.floor,
        max_height: ceiling,
    }
}

#[cfg(test)]
#[path = "stack_tests.rs"]
mod tests;
