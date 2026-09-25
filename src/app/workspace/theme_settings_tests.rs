use super::*;
use crate::app::ui::{
    file_icons,
    theme::{ColorKey, SyntaxKey, ThemeToken, builtin::BUILT_IN_THEMES},
};

fn saved_library() -> String {
    let mut library = ThemeLibrary::default();
    let mut definition = BUILT_IN_THEMES[2].clone();
    definition.name = "Ocean".to_owned();
    definition.colors.canvas = parse_hex("#123456").expect("hex");
    definition.set_color(
        ThemeToken::Syntax(SyntaxKey::keyword),
        parse_hex("#654321").expect("hex"),
    );
    definition.set_color(ThemeToken::Icon(0), parse_hex("#abcdef").expect("hex"));
    library.upsert(definition).expect("save theme");
    library.to_css()
}

#[test]
fn themes_fall_back_to_the_bundled_theme() {
    let settings = ThemeSettings::load(None, None);
    assert!(settings.editable());
    assert_eq!(settings.library.selected_name(), BUILT_IN_THEMES[0].name);
    assert!(settings.draft.is_none());
    assert!(settings.error.is_none());
}

#[test]
fn saved_themes_and_selection_are_restored() {
    let settings = ThemeSettings::load(Some(&saved_library()), Some("Ocean"));
    assert!(settings.editable());
    assert_eq!(settings.library.selected_name(), "Ocean");
    assert_eq!(
        settings.library.selected().colors.get(ColorKey::canvas),
        parse_hex("#123456").expect("hex")
    );
    assert_eq!(settings.library.user_themes().len(), 1);
    assert!(settings.error.is_none());
}

#[test]
fn saved_icon_and_syntax_tokens_are_restored() {
    let settings = ThemeSettings::load(Some(&saved_library()), Some("Ocean"));
    let definition = settings.library.selected();
    assert_eq!(
        definition.color(ThemeToken::Syntax(SyntaxKey::keyword)),
        parse_hex("#654321").expect("hex")
    );
    assert_eq!(
        definition.color(ThemeToken::Icon(0)),
        parse_hex("#abcdef").expect("hex")
    );
    assert_eq!(
        definition.color(ThemeToken::Icon(1)),
        file_icons::native_color(1)
    );
}

#[test]
fn unreadable_saved_themes_keep_the_store_untouched() {
    let settings = ThemeSettings::load(Some("{ not css"), None);
    assert!(!settings.editable());
    assert!(settings.error.is_some());
    assert_eq!(settings.library.selected_name(), BUILT_IN_THEMES[0].name);
}

#[test]
fn every_bundled_theme_can_be_selected() {
    let mut settings = ThemeSettings::default();
    for definition in BUILT_IN_THEMES.iter() {
        settings
            .library
            .select(&definition.name)
            .expect("select bundled theme");
        assert_eq!(settings.library.selected(), *definition);
    }
}
