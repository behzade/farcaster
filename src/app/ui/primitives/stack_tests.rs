use super::*;
use crate::app::ui::theme::theme;
use gpui::px;

const FLOORS: [f32; 2] = [60.0, 76.0];
const BUDGET: f32 = 599.0;

fn list(values: &[f32]) -> Vec<Pixels> {
    values.iter().copied().map(px).collect()
}

#[test]
fn the_panels_share_what_the_chat_list_leaves_over() {
    assert_eq!(panel_space(px(699.0), px(128.0)), px(571.0));
    assert_eq!(panel_space(px(100.0), px(128.0)), px(0.0));
}

#[test]
fn a_layout_keeps_every_size_that_fits_and_charges_the_rest_to_the_bottom_panel() {
    let floors = list(&FLOORS);
    let room = |sizes: &[f32], budget: f32, index: usize| {
        panel_room(&list(sizes), &floors, px(budget), index, false)
    };
    assert_eq!(room(&[100.0, 400.0], BUDGET, 0), px(100.0));
    assert_eq!(room(&[100.0, 400.0], BUDGET, 1), px(400.0));
    assert_eq!(room(&[523.0, 539.0], BUDGET, 0), px(523.0));
    assert_eq!(room(&[523.0, 539.0], BUDGET, 1), px(76.0));
    assert_eq!(room(&[523.0, 539.0], 300.0, 0), px(224.0));
    assert_eq!(room(&[523.0, 539.0], 300.0, 1), px(76.0));
    assert_eq!(room(&[523.0, 539.0], 0.0, 1), px(76.0));
}

#[test]
fn a_drag_takes_its_height_off_the_panels_above_it() {
    let floors = list(&FLOORS);
    let dragged = panel_resized(&list(&[100.0, 200.0]), &floors, px(BUDGET), 1, px(539.0));
    assert_eq!(dragged, list(&[60.0, 539.0]));
    assert!(dragged.iter().fold(px(0.0), |total, h| total + *h) <= px(BUDGET));
    assert_eq!(
        panel_resized(&list(&[60.0, 200.0]), &floors, px(BUDGET), 1, px(4000.0)),
        list(&[60.0, 539.0]),
    );
}

#[test]
fn a_drag_of_a_panel_leaves_the_one_below_it_alone() {
    let dragged = panel_resized(
        &list(&[100.0, 200.0]),
        &list(&FLOORS),
        px(BUDGET),
        0,
        px(4000.0),
    );
    assert_eq!(dragged, list(&[399.0, 200.0]));
}

#[test]
fn a_panel_opens_at_its_preferred_height_inside_its_room() {
    let slot = PanelSlot::new(px(76.0), px(160.0));
    assert_eq!(panel_bounds(px(200.0), slot).height, px(160.0));
    assert_eq!(panel_bounds(px(200.0), slot).max_height, px(200.0));
    assert_eq!(panel_bounds(px(40.0), slot).max_height, px(76.0));

    let header = theme().controls.icon_button;
    let collapsed = PanelSlot::new(header, header);
    assert_eq!(panel_bounds(px(472.0), collapsed).height, header);
}
