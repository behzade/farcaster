//! Gruvbox dark-hard visual tokens for the complete native surface.

use gpui::{App, Pixels, Rgba, px};
use gpui_component::{
    highlighter::HighlightTheme,
    theme::{Theme as ComponentTheme, ThemeMode, ThemeTokens},
};

pub(crate) const UI_FONT_FAMILY: &str = ".SystemUIFont";
pub(crate) const MONO_FONT_FAMILY: &str = "Lilex";

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub colors: Colors,
    pub space: Space,
    pub type_scale: TypeScale,
    pub icons: IconScale,
    pub controls: ControlScale,
    pub radius: Pixels,
    pub border: Pixels,
    pub layout: Layout,
}

#[derive(Clone, Copy)]
pub(crate) struct Colors {
    pub canvas: Rgba,
    pub panel: Rgba,
    pub surface: Rgba,
    pub hover: Rgba,
    pub selection: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub subtle: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_active: Rgba,
    pub link: Rgba,
    pub code: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    pub success: Rgba,
    pub diff_added: Rgba,
    pub diff_deleted: Rgba,
    pub backdrop: Rgba,
}

#[derive(Clone, Copy)]
pub(crate) struct Space {
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct TypeScale {
    pub caption: Pixels,
    pub body_small: Pixels,
    pub body: Pixels,
    pub display: Pixels,
    pub line_body: Pixels,
    pub line_composer: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct IconScale {
    pub inline: Pixels,
    pub control: Pixels,
    pub prominent: Pixels,
}

#[derive(Clone, Copy)]
pub(crate) struct ControlScale {
    pub icon_button: Pixels,
    pub utility_row: Pixels,
    pub project_row: Pixels,
    pub archived_preview_row: Pixels,
    pub agent_marker: Pixels,
}
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub window_width: Pixels,
    pub window_height: Pixels,
    pub session_rail: Pixels,
    pub session_rail_min: Pixels,
    pub session_rail_max: Pixels,
    pub run_panel: Pixels,
    pub run_panel_min: Pixels,
    pub run_panel_max: Pixels,
    pub transcript_overdraw: Pixels,
    pub composer_min: Pixels,
    pub dialog_width: Pixels,
    pub dialog_max_height: Pixels,
    pub tool_max_height: Pixels,
    pub session_row_height: Pixels,
    pub status_row_height: Pixels,
}

const fn rgb(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}
const fn rgba(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 24) & 0xff) as f32 / 255.0,
        g: ((hex >> 16) & 0xff) as f32 / 255.0,
        b: ((hex >> 8) & 0xff) as f32 / 255.0,
        a: (hex & 0xff) as f32 / 255.0,
    }
}

pub(crate) const THEME: Theme = Theme {
    colors: Colors {
        canvas: rgb(0x1d2021),
        panel: rgb(0x282828),
        surface: rgb(0x3c3836),
        hover: rgb(0x504945),
        selection: rgba(0x8ec07c33),
        border: rgb(0x665c54),
        text: rgb(0xd5c4a1),
        muted: rgb(0xa89984),
        subtle: rgb(0x928374),
        accent: rgb(0x8ec07c),
        accent_hover: rgb(0xb8bb26),
        accent_active: rgb(0x689d6a),
        link: rgb(0x83a598),
        code: rgb(0xd79921),
        warning: rgb(0xfabd2f),
        error: rgb(0xfb4934),
        success: rgb(0xb8bb26),
        diff_added: rgba(0xb8bb2626),
        diff_deleted: rgba(0xfb493426),
        backdrop: rgba(0x000000b3),
    },
    space: Space {
        xs: px(4.0),
        sm: px(8.0),
        md: px(16.0),
    },
    type_scale: TypeScale {
        caption: px(12.0),
        body_small: px(13.0),
        body: px(14.0),
        display: px(18.0),
        line_body: px(20.0),
        line_composer: px(22.0),
    },
    icons: IconScale {
        inline: px(16.0),
        control: px(18.0),
        prominent: px(20.0),
    },
    controls: ControlScale {
        icon_button: px(28.0),
        utility_row: px(40.0),
        project_row: px(36.0),
        archived_preview_row: px(56.0),
        agent_marker: px(20.0),
    },
    radius: px(6.0),
    border: px(1.0),
    layout: Layout {
        window_width: px(1240.0),
        window_height: px(820.0),
        session_rail: px(272.0),
        session_rail_min: px(248.0),
        session_rail_max: px(304.0),
        run_panel: px(312.0),
        run_panel_min: px(288.0),
        run_panel_max: px(344.0),
        transcript_overdraw: px(160.0),
        composer_min: px(80.0),
        dialog_width: px(560.0),
        dialog_max_height: px(680.0),
        tool_max_height: px(220.0),
        session_row_height: px(64.0),
        status_row_height: px(24.0),
    },
};

pub(crate) fn install_component_theme(cx: &mut App) {
    ComponentTheme::change(ThemeMode::Dark, None, cx);
    let theme = ComponentTheme::global_mut(cx);
    theme.font_family = UI_FONT_FAMILY.into();
    theme.font_size = THEME.type_scale.body;
    theme.mono_font_family = MONO_FONT_FAMILY.into();
    theme.mono_font_size = THEME.type_scale.body_small;
    theme.highlight_theme = HighlightTheme::default_dark();
    theme.radius = THEME.radius;
    theme.radius_lg = THEME.radius;
    theme.shadow = true;
    let colors = &mut theme.colors;
    colors.background = THEME.colors.canvas.into();
    colors.foreground = THEME.colors.text.into();
    colors.accent = THEME.colors.surface.into();
    colors.accent_foreground = THEME.colors.text.into();
    colors.link = THEME.colors.link.into();
    colors.link_active = THEME.colors.link.into();
    colors.link_hover = THEME.colors.link.into();
    colors.border = THEME.colors.border.into();
    colors.input = THEME.colors.border.into();
    colors.muted = THEME.colors.panel.into();
    colors.muted_foreground = THEME.colors.muted.into();
    colors.popover = THEME.colors.panel.into();
    colors.popover_foreground = THEME.colors.text.into();
    colors.primary = THEME.colors.surface.into();
    colors.primary_hover = THEME.colors.hover.into();
    colors.primary_active = THEME.colors.hover.into();
    colors.primary_foreground = THEME.colors.text.into();
    colors.secondary = THEME.colors.surface.into();
    colors.secondary_hover = THEME.colors.hover.into();
    colors.secondary_active = THEME.colors.hover.into();
    colors.secondary_foreground = THEME.colors.text.into();
    colors.button = THEME.colors.surface.into();
    colors.button_hover = THEME.colors.hover.into();
    colors.button_active = THEME.colors.hover.into();
    colors.button_foreground = THEME.colors.text.into();
    colors.button_primary = THEME.colors.accent.into();
    colors.button_primary_hover = THEME.colors.accent_hover.into();
    colors.button_primary_active = THEME.colors.accent_active.into();
    colors.button_primary_foreground = THEME.colors.canvas.into();
    colors.danger = THEME.colors.error.into();
    colors.danger_foreground = THEME.colors.canvas.into();
    colors.warning = THEME.colors.warning.into();
    colors.warning_foreground = THEME.colors.canvas.into();
    colors.success = THEME.colors.success.into();
    colors.success_foreground = THEME.colors.canvas.into();
    colors.ring = THEME.colors.accent.into();
    colors.caret = THEME.colors.text.into();
    colors.selection = THEME.colors.selection.into();
    theme.tokens = ThemeTokens::from(theme.colors);
}

#[cfg(test)]
mod tests {
    use super::THEME;

    #[test]
    fn sidebar_widths_match_the_design_bounds() {
        assert_eq!(f32::from(THEME.layout.session_rail), 272.0);
        assert_eq!(f32::from(THEME.layout.session_rail_min), 248.0);
        assert_eq!(f32::from(THEME.layout.session_rail_max), 304.0);
        assert_eq!(f32::from(THEME.layout.run_panel), 312.0);
        assert_eq!(f32::from(THEME.layout.run_panel_min), 288.0);
        assert_eq!(f32::from(THEME.layout.run_panel_max), 344.0);
    }

    #[test]
    fn icon_and_control_tokens_keep_icons_optically_proportional() {
        assert!(THEME.icons.inline >= THEME.type_scale.body);
        assert!(THEME.icons.control > THEME.icons.inline);
        assert!(THEME.icons.prominent >= THEME.icons.control);
        assert!(THEME.controls.icon_button > THEME.icons.prominent);
    }
}
