use super::*;
use gpui::px;

fn sizes(values: &[f32]) -> Vec<Pixels> {
    values.iter().copied().map(px).collect()
}

fn floors(values: &[f32]) -> Vec<Pixels> {
    values.iter().copied().map(px).collect()
}

#[test]
fn the_panels_share_what_the_chat_list_leaves_over() {
    assert_eq!(panel_space(px(699.0), px(128.0)), px(571.0));
    assert_eq!(
        panel_space(px(100.0), px(128.0)),
        px(0.0),
        "a stack shorter than the chat list's floor leaves the panels nothing"
    );
}

#[test]
fn panels_that_fit_keep_the_height_each_one_remembers() {
    let heights = panel_heights(&sizes(&[100.0, 400.0]), &floors(&[60.0, 76.0]), px(599.0));
    assert_eq!(heights, sizes(&[100.0, 400.0]));
}

#[test]
fn a_shortfall_comes_off_the_bottom_panel_first() {
    let heights = panel_heights(&sizes(&[523.0, 539.0]), &floors(&[60.0, 76.0]), px(599.0));
    assert_eq!(
        heights,
        sizes(&[523.0, 76.0]),
        "the history gives up its height down to its floor, the archive is left where it was"
    );
}

#[test]
fn a_shortfall_deeper_than_one_panel_takes_the_next_one_up() {
    let heights = panel_heights(&sizes(&[523.0, 539.0]), &floors(&[60.0, 76.0]), px(300.0));
    assert_eq!(heights, sizes(&[224.0, 76.0]));
}

#[test]
fn no_shortfall_ever_takes_a_panel_under_its_floor() {
    let heights = panel_heights(&sizes(&[523.0, 539.0]), &floors(&[60.0, 76.0]), px(0.0));
    assert_eq!(heights, sizes(&[60.0, 76.0]));
}

#[test]
fn a_drag_takes_the_height_off_the_panel_above_it() {
    let (panels, floors) = (sizes(&[100.0, 200.0]), floors(&[60.0, 76.0]));
    let dragged = panel_resized(&panels, &floors, px(599.0), 1, px(539.0));
    assert_eq!(
        dragged,
        sizes(&[60.0, 539.0]),
        "the history takes the archive down to its floor and then the chat list's spare room"
    );
    assert!(
        dragged.iter().fold(px(0.0), |total, h| total + *h) <= px(599.0),
        "and the panels still fit the space the chat list leaves them"
    );
}

#[test]
fn a_drag_stops_at_the_floor_of_the_panel_above() {
    let (panels, floors) = (sizes(&[60.0, 200.0]), floors(&[60.0, 76.0]));
    let dragged = panel_resized(&panels, &floors, px(599.0), 1, px(4000.0));
    assert_eq!(
        dragged,
        sizes(&[60.0, 539.0]),
        "with the archive already at its floor the drag stops where the space runs out"
    );
}

#[test]
fn dragging_a_panel_never_resizes_the_one_below_it() {
    let (panels, floors) = (sizes(&[100.0, 200.0]), floors(&[60.0, 76.0]));
    let dragged = panel_resized(&panels, &floors, px(599.0), 0, px(4000.0));
    assert_eq!(
        dragged[1],
        px(200.0),
        "the archive's own edge only moves against the chat list, never against the history"
    );
    assert_eq!(
        dragged[0],
        px(399.0),
        "and it stops where the history's floor begins"
    );
}

#[test]
fn a_drag_is_measured_against_whoever_is_above_the_edge() {
    let (panels, floors) = (sizes(&[100.0, 200.0]), floors(&[60.0, 76.0]));
    assert_eq!(
        panel_max(&panels, &floors, px(599.0), 1),
        px(539.0),
        "the history can reach the archive's floor, no further"
    );
    assert_eq!(
        panel_max(&panels, &floors, px(599.0), 0),
        px(399.0),
        "the archive leaves the history exactly the height it holds"
    );
}

#[test]
fn a_panel_that_was_never_measured_is_left_unnumbered() {
    let slot = PanelSlot::new(px(76.0), px(160.0));
    assert_eq!(
        panel_bounds(None, slot).max_height,
        theme().layout.notice_panel_max
    );
    let measured = panel_bounds(Some(px(200.0)), slot);
    assert_eq!(measured.max_height, px(200.0));
    assert_eq!(measured.height, px(160.0));
    assert_eq!(measured.row_height, None, "a drag steps through no rows");
    assert_eq!(
        panel_bounds(Some(px(40.0)), slot).max_height,
        px(76.0),
        "a room under the floor is the floor"
    );
}

#[test]
fn a_minimized_panel_is_its_header_however_tall_it_was_before() {
    let header = theme().controls.icon_button;
    let collapsed = PanelSlot::new(header, header);
    assert_eq!(panel_bounds(Some(px(472.0)), collapsed).height, header);
    assert_eq!(panel_bounds(None, collapsed).height, header);
    assert!(
        panel_bounds(Some(px(472.0)), PanelSlot::new(px(76.0), px(160.0))).height > header,
        "and an open one asks for more than its header"
    );
}
