use gpui::App;
use std::ops::Deref;

mod async_util;
mod component_traits;
mod element_ext;
pub mod global_state;
mod icon;
mod index_path;
mod root;
mod sizing;
mod styled;
mod window_border;

pub(crate) mod actions {
    pub use gpui_base::actions::*;
}

pub mod button;
pub mod highlighter;
pub mod input;
pub mod kbd;
mod label;
pub mod list;
pub mod menu;
pub mod native_menu;
pub mod popover;
pub mod scroll;
pub mod select;
mod skeleton;
mod spinner;
pub mod text;
pub mod theme;
pub mod tooltip;

pub use crate::Disableable;
pub use element_ext::*;
pub use global_state::GlobalState;
pub use gpui_base::animation;
pub use gpui_base::{
    AxisExt, Edges, FocusTrapElement, InteractiveElementExt, LengthExt, Measure, OngoingScrollExt,
    Placement, Side, measure, measure_if,
};
pub use gpui_base::{VirtualList, VirtualListScrollHandle, h_virtual_list, v_virtual_list};
pub use icon::*;
pub use index_path::IndexPath;
pub use input::{Rope, RopeExt, RopeLines};
pub use root::Root;
pub use styled::*;
pub use theme::*;
pub use window_border::{WindowBorder, window_border, window_paddings};

rust_i18n::i18n!("locales", fallback = "en");

pub fn init(cx: &mut App) {
    theme::init(cx);
    global_state::init(cx);
    root::init(cx);
    gpui_base::init(cx);
    list::init(cx);
    popover::init(cx);
    menu::init(cx);
    text::init(cx);
    tooltip::init(cx);
}

#[inline]
pub fn locale() -> impl Deref<Target = str> {
    rust_i18n::locale()
}

#[inline]
pub fn set_locale(locale: &str) {
    rust_i18n::set_locale(locale)
}
