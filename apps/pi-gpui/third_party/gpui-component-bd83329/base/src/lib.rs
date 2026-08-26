//! Extracted behavior and infrastructure foundations used by Pi GPUI.

pub mod actions;
pub mod animation;
#[doc(hidden)]
pub mod async_util;
mod auto_scroll;
mod button;
pub mod component_traits;
mod element_ext;
mod event;
mod focus_trap;
mod geometry;
mod global_state;
mod history;
mod index_path;
pub mod input;
mod list_settings;
#[cfg(all(target_os = "macos", not(test)))]
mod macos_accessibility;
mod measure;
mod number_input;
mod popover;
mod popup;
mod positioner;
mod scrollbar;
mod state_style;
mod styled;
mod text_boundary;
mod text_selection;
mod theme;
pub mod theme_tokens;
mod tooltip;
mod virtual_list;

pub use auto_scroll::AutoScroll;
pub use button::{Button, ButtonStyles};
pub use component_traits::FocusableExt;
pub use component_traits::{Disableable, Selectable};
pub use element_ext::ElementExt;
pub use event::{InteractiveElementExt, OngoingScrollExt};
pub use focus_trap::FocusTrapElement;
#[doc(hidden)]
pub use focus_trap::active_focus_trap;
pub use geometry::*;
pub use global_state::GlobalState;
pub use history::{History, HistoryItem};
pub use index_path::IndexPath;
pub use input::{Editor, Input, InputBase, InputStyles, Textarea};
pub use list_settings::ListSettings;
#[cfg(all(target_os = "macos", not(test)))]
#[doc(hidden)]
pub use macos_accessibility::install_window_hit_test_forwarder;
#[doc(hidden)]
pub use measure::measurement_enabled;
pub use measure::{Measure, measure, measure_if};
pub use number_input::{NumberInputEvent, NumberStep, StepAction};
pub use popover::{Popover, PopoverState};
pub use popup::{POPUP_PRIORITY, Popup};
pub use positioner::{Align, Positioner, ResolvedPosition};
pub use scrollbar::{
    Scrollbar, ScrollbarAxis, ScrollbarHandle, ScrollbarMode, ScrollbarStyles, ScrollbarThumbStyle,
    ScrollbarTrackStyle,
};
pub use state_style::StateStyle;
#[cfg(any(feature = "inspector", debug_assertions))]
pub use styled::styled_ext_reflection_methods;
pub use styled::{RoleOverride, StyledExt, box_shadow, h_flex, v_flex};
pub use text_selection::{
    TextSelection, TextSelectionContentKey, TextSelectionCoverage, TextSelectionEndpoint,
    TextSelectionEvent, TextSelectionHandle, TextSelectionLayer, TextSelectionProjection,
    TextSelectionRegistration, TextSelectionRun, TextSelectionScopeId, TextSelectionSnapshot,
    TextSelectionWindowPoints,
};
pub use theme::{ResizableTheme, ScrollbarTheme, Theme};
pub use theme_tokens::{
    ColorTokens, RadiusTokens, SemanticThemeTokens, ShadowTokens, SpacingTokens, TextStyleToken,
    TypographyTokens,
};
pub use tooltip::{Tooltip, TooltipOverlay, TooltipPositioner, TooltipRequest, TooltipTransition};
#[doc(hidden)]
pub use virtual_list::virtual_list;
pub use virtual_list::{VirtualList, VirtualListScrollHandle, h_virtual_list, v_virtual_list};

use gpui::App;

pub fn init(cx: &mut App) {
    let _ = Theme::global_mut(cx);
    GlobalState::init(cx);
    focus_trap::init(cx);
    popover::init(cx);
    input::init(cx);
}
