use std::sync::Arc;

use gpui::Rgba;
use gpui_component::highlighter::{
    FontStyle, HighlightTheme, HighlightThemeStyle, StatusColors, SyntaxColors, ThemeStyle,
};

use crate::app::ui::theme::{Appearance, ColorKey, Colors, Theme, color_hex};

#[derive(Clone, Copy)]
enum SyntaxKind {
    Plain,
    Italic,
    Bold,
}

fn syntax_style(kind: SyntaxKind, color: Rgba) -> Option<ThemeStyle> {
    Some(ThemeStyle {
        color: Some(color.into()),
        font_style: match kind {
            SyntaxKind::Italic => Some(FontStyle::Italic),
            SyntaxKind::Plain | SyntaxKind::Bold => None,
        },
        font_weight: match kind {
            SyntaxKind::Bold => Some(gpui_component::highlighter::FontWeightContent::Semibold),
            SyntaxKind::Plain | SyntaxKind::Italic => None,
        },
    })
}

macro_rules! syntax_keys {
    ($($field:ident => $name:literal : $palette:ident : $kind:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        #[allow(non_camel_case_types)]
        pub(crate) enum SyntaxKey {
            $($field),*
        }

        impl SyntaxKey {
            pub(crate) const ALL: &'static [SyntaxKey] = &[$(SyntaxKey::$field),*];

            pub(crate) fn from_name(name: &str) -> Option<SyntaxKey> {
                match name {
                    $($name => Some(SyntaxKey::$field),)*
                    _ => None,
                }
            }

            pub(crate) fn name(self) -> &'static str {
                match self {
                    $(SyntaxKey::$field => $name),*
                }
            }

            pub(crate) fn label(self) -> String {
                crate::app::ui::theme::label(self.name())
            }

            pub(crate) fn default_color(self, colors: Colors) -> Rgba {
                colors.get(match self {
                    $(SyntaxKey::$field => ColorKey::$palette),*
                })
            }

            pub(crate) fn tint(self, syntax: &mut SyntaxColors, color: Rgba) {
                let style: &mut Option<ThemeStyle> = match self {
                    $(SyntaxKey::$field => &mut syntax.$field),*
                };
                let existing = style.get_or_insert(ThemeStyle {
                    color: None,
                    font_style: None,
                    font_weight: None,
                });
                existing.color = Some(color.into());
            }
        }

        fn default_syntax(colors: Colors) -> SyntaxColors {
            SyntaxColors {
                $($field: syntax_style(SyntaxKind::$kind, colors.get(ColorKey::$palette))),*
            }
        }
    };
}

syntax_keys!(
    attribute => "attribute" : code : Plain,
    boolean => "boolean" : code : Plain,
    comment => "comment" : subtle : Italic,
    comment_doc => "comment-doc" : muted : Italic,
    constant => "constant" : code : Plain,
    constructor => "constructor" : skill : Plain,
    embedded => "embedded" : accent : Plain,
    emphasis => "emphasis" : text : Italic,
    emphasis_strong => "emphasis-strong" : text : Bold,
    enum_ => "enum" : file : Plain,
    function => "function" : link : Plain,
    hint => "hint" : muted : Plain,
    keyword => "keyword" : accent : Plain,
    label => "label" : file : Plain,
    link_text => "link-text" : link : Plain,
    link_uri => "link-uri" : link : Plain,
    number => "number" : code : Plain,
    operator => "operator" : muted : Plain,
    predictive => "predictive" : subtle : Plain,
    preproc => "preproc" : skill : Plain,
    primary => "primary" : text : Plain,
    property => "property" : file : Plain,
    punctuation => "punctuation" : muted : Plain,
    punctuation_bracket => "punctuation-bracket" : muted : Plain,
    punctuation_delimiter => "punctuation-delimiter" : muted : Plain,
    punctuation_list_marker => "punctuation-list-marker" : accent : Plain,
    punctuation_special => "punctuation-special" : accent : Plain,
    string => "string" : success : Plain,
    string_escape => "string-escape" : warning : Plain,
    string_regex => "string-regex" : skill : Plain,
    string_special => "string-special" : warning : Plain,
    string_special_symbol => "string-special-symbol" : warning : Plain,
    tag => "tag" : file : Plain,
    tag_doctype => "tag-doctype" : subtle : Plain,
    text_code_span => "text-code-span" : code : Plain,
    text_literal => "text-literal" : code : Plain,
    title => "title" : text : Plain,
    type_ => "type" : skill : Plain,
    variable => "variable" : text : Plain,
    variable_special => "variable-special" : accent : Plain,
    variant => "variant" : skill : Plain,
);

pub(crate) fn for_theme(
    theme: Theme,
    appearance: Appearance,
    overrides: &[(SyntaxKey, Rgba)],
) -> Arc<HighlightTheme> {
    let colors = theme.colors;
    let mut syntax = default_syntax(colors);
    for (key, color) in overrides {
        key.tint(&mut syntax, *color);
    }
    Arc::new(HighlightTheme {
        name: format!("Farcaster {}", color_hex(colors.canvas)),
        appearance: appearance.mode(),
        style: HighlightThemeStyle {
            editor_background: Some(colors.canvas.into()),
            editor_foreground: Some(colors.text.into()),
            editor_active_line: Some(colors.highlight.into()),
            editor_line_number: Some(colors.subtle.into()),
            editor_active_line_number: Some(colors.text.into()),
            editor_invisible: Some(colors.surface.into()),
            editor_gutter_background: Some(colors.panel.into()),
            status: StatusColors::default(),
            syntax,
        },
    })
}
