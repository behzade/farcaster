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

pub(crate) fn panel_room(
    sizes: &[Pixels],
    floors: &[Pixels],
    budget: Pixels,
    index: usize,
    resizing: bool,
) -> Pixels {
    if resizing {
        return panel_max(sizes, floors, budget, index);
    }
    panel_heights(sizes, floors, budget)[index]
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

fn panel_heights(sizes: &[Pixels], floors: &[Pixels], budget: Pixels) -> Vec<Pixels> {
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

fn panel_max(sizes: &[Pixels], floors: &[Pixels], budget: Pixels, index: usize) -> Pixels {
    let above = floors[..index].iter().fold(px(0.0), |total, f| total + *f);
    let below = sizes[index + 1..]
        .iter()
        .fold(px(0.0), |total, s| total + *s);
    (budget - above - below).max(floors[index])
}

fn at_least_floors(sizes: &[Pixels], floors: &[Pixels]) -> Vec<Pixels> {
    sizes
        .iter()
        .zip(floors)
        .map(|(size, floor)| (*size).max(*floor))
        .collect()
}

#[cfg(test)]
#[path = "stack_tests.rs"]
mod tests;
