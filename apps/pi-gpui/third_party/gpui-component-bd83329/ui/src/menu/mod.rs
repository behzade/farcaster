use gpui::App;

mod dropdown_menu;
mod menu_item;
mod popup_menu;

pub use dropdown_menu::DropdownMenu;
pub use popup_menu::{PopupMenu, PopupMenuItem};

pub(crate) fn init(cx: &mut App) {
    popup_menu::init(cx);
}
