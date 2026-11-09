use super::*;
use crate::app::ui::theme::theme;
use gpui::px;

#[test]
fn the_panels_share_what_the_chat_list_leaves_over() {
    assert_eq!(panel_space(px(699.0), px(128.0)), px(571.0));
    assert_eq!(panel_space(px(100.0), px(128.0)), px(0.0));
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
