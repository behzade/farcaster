use super::*;
use crate::app::ui::{
    file_icons,
    theme::{LengthKey, Pixels, SyntaxKey, ThemeToken, builtin::BUILT_IN_THEMES, parse_hex},
};

fn pxf(value: f32) -> Pixels {
    gpui::px(value)
}

fn icon_index(name: &str) -> usize {
    file_icons::ICON_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .expect("bundled icon")
}

fn default_name() -> &'static str {
    &BUILT_IN_THEMES[0].name
}

fn default_colors() -> Colors {
    BUILT_IN_THEMES[0].colors
}

fn definition(name: &str) -> ThemeDefinition {
    ThemeDefinition {
        name: name.to_owned(),
        appearance: Appearance::Dark,
        colors: default_colors(),
        tokens: Vec::new(),
        lengths: Vec::new(),
    }
}

fn css(name: &str) -> String {
    definition(name).to_css().expect("encode theme")
}

fn custom_library() -> ThemeLibrary {
    let mut library = ThemeLibrary::default();
    library.upsert(definition("Ocean")).expect("add theme");
    library
}

#[test]
fn built_in_themes_are_listed_before_user_themes() {
    let mut library = custom_library();
    library.upsert(definition("Dusk")).expect("add theme");
    let names = library
        .display_order()
        .into_iter()
        .map(|theme| theme.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![default_name(), "White", "Black", "Ocean", "Dusk"]
    );
}

#[test]
fn selecting_an_unknown_theme_is_rejected() {
    let mut library = ThemeLibrary::default();
    assert!(library.select("Missing").is_err());
    assert_eq!(library.selected_name(), default_name());
    library.select("White").expect("select built-in");
    assert_eq!(library.selected().appearance, Appearance::Light);
}

#[test]
fn built_in_names_are_reserved_for_user_edits() {
    let mut library = ThemeLibrary::default();
    assert!(library.upsert(definition("White")).is_err());
    assert!(library.rename("Ocean", "White").is_err());
    assert!(library.remove("White").is_err());
    assert_eq!(library.user_themes().len(), 0);
}

#[test]
fn upserting_an_existing_name_replaces_it() {
    let mut library = custom_library();
    let mut recolored = definition("Ocean");
    recolored.colors.canvas = parse_hex("#010203").expect("hex");
    library.upsert(recolored).expect("replace theme");
    assert_eq!(library.user_themes().len(), 1);
    assert_eq!(
        library.find("Ocean").expect("theme").colors.canvas,
        parse_hex("#010203").expect("hex")
    );
}

#[test]
fn renaming_a_theme_carries_the_selection() {
    let mut library = custom_library();
    library.select("Ocean").expect("select theme");
    library.rename("Ocean", "Lagoon").expect("rename theme");
    assert_eq!(library.selected_name(), "Lagoon");
    assert!(library.find("Ocean").is_none());
    assert!(library.rename("Lagoon", "Lagoon").is_ok());
}

#[test]
fn removing_the_selected_theme_falls_back_to_the_default() {
    let mut library = custom_library();
    library.select("Ocean").expect("select theme");
    library.remove("Ocean").expect("remove theme");
    assert_eq!(library.selected_name(), default_name());
    assert!(library.user_themes().is_empty());
}

#[test]
fn importing_renames_a_theme_instead_of_clobbering_one() {
    let mut library = custom_library();
    let css = css("Ocean");
    assert_eq!(
        library.import(&css).expect("import theme"),
        vec!["Ocean 2".to_owned()]
    );
    assert_eq!(
        library.import(&css).expect("import theme"),
        vec!["Ocean 3".to_owned()]
    );
    assert_eq!(library.user_themes().len(), 3);
}

#[test]
fn imported_themes_can_carry_several_blocks() {
    let mut library = ThemeLibrary::default();
    let css = format!("{}\n{}", css("Dusk"), css("Dawn"));
    assert_eq!(
        library.import(&css).expect("import themes"),
        vec!["Dusk".to_owned(), "Dawn".to_owned()]
    );
    assert_eq!(library.user_themes().len(), 2);
}

#[test]
fn imported_theme_files_reject_broken_content() {
    let mut library = ThemeLibrary::default();
    assert!(library.import("").is_err());
    assert!(library.import("not css").is_err());
    assert!(library.import(":root[data-theme=\"Ocean\"] {}").is_err());

    let broken = css("Ocean").replace("--canvas: #1b1f20;", "--canvas: #zzzzzz;");
    assert!(library.import(&broken).is_err());

    let unknown = css("Ocean").replace(
        "  --canvas: #1b1f20;\n",
        "  --canvas: #1b1f20;\n  --oops: #000000;\n",
    );
    assert!(library.import(&unknown).is_err());

    let missing = css("Ocean").replace("  --canvas: #1b1f20;\n", "");
    assert!(library.import(&missing).is_err());

    let unitless =
        css("Ocean").replace("--canvas: #1b1f20;", "--canvas: #1b1f20;\n  --space-xs: 4;");
    assert!(library.import(&unitless).is_err());

    let no_color = css("Ocean").replace("--canvas: #1b1f20;", "--canvas: 4px;");
    assert!(library.import(&no_color).is_err());
}

#[test]
fn icon_and_syntax_tokens_round_trip_through_css() {
    let mut definition = definition("Ocean");
    definition.set_color(
        ThemeToken::Icon(icon_index("rust")),
        parse_hex("#ff0000").expect("hex"),
    );
    definition.set_color(
        ThemeToken::Syntax(SyntaxKey::keyword),
        parse_hex("#00ff00").expect("hex"),
    );
    definition.set_length(LengthKey::from_name("space-xs").expect("token"), pxf(6.0));
    definition.set_length(LengthKey::from_name("size-24").expect("token"), pxf(30.0));
    let css = definition.to_css().expect("encode theme");
    assert!(css.contains("--icon-rust: #ff0000;"));
    assert!(css.contains("--keyword: #00ff00;"));
    assert!(css.contains("--space-xs: 6px;"));
    assert!(css.contains("--size-24: 30px;"));
    assert_eq!(
        ThemeDefinition::from_css(&css).expect("decode theme"),
        definition
    );
}

#[test]
fn unset_tokens_fall_back_to_the_palette_and_the_bundled_icons() {
    let definition = definition("Ocean");
    assert_eq!(
        definition.color(ThemeToken::Syntax(SyntaxKey::keyword)),
        definition.colors.accent
    );
    assert_eq!(
        definition.color(ThemeToken::Icon(icon_index("rust"))),
        file_icons::native_color(icon_index("rust"))
    );
    assert!(!definition.is_custom(ThemeToken::Syntax(SyntaxKey::keyword)));
}

#[test]
fn theme_css_round_trips() {
    let definition = definition("Ocean");
    let css = definition.to_css().expect("encode theme");
    assert_eq!(
        ThemeDefinition::from_css(&css).expect("decode theme"),
        ThemeDefinition {
            name: "Ocean".to_owned(),
            appearance: Appearance::Dark,
            ..definition
        }
    );
}

#[test]
fn libraries_round_trip_through_css() {
    let mut library = custom_library();
    library.select("Ocean").expect("select theme");
    let css = library.to_css();
    assert_eq!(
        ThemeLibrary::from_css(&css, Some("Ocean")).expect("decode library"),
        library
    );
    assert_eq!(
        ThemeLibrary::from_css(&css, None)
            .expect("decode library")
            .selected_name(),
        default_name()
    );
}

#[test]
fn normalization_drops_unusable_saved_themes() {
    let css = format!(
        "{}\n{}\n{}",
        css("Black"),
        css("Dusk"),
        css("Dusk").replace("data-theme=\"Dusk\"", "data-theme=\"Dusk \""),
    );
    let library = ThemeLibrary::from_css(&css, Some("Missing")).expect("decode library");
    assert_eq!(library.selected_name(), default_name());
    assert_eq!(library.user_themes().len(), 1);
    assert!(library.is_user_theme("Dusk"));
}

#[test]
fn exported_themes_use_css_tokens() {
    let mut library = custom_library();
    library.select("Black").expect("select built-in");
    let css = library.export("Black").expect("export theme");
    assert!(css.contains(":root[data-theme=\"Black\"][data-appearance=\"dark\"] {"));
    assert!(css.contains("--canvas: #000000;"));
}

#[test]
fn theme_names_are_trimmed_and_bounded() {
    assert_eq!(validate_theme_name("  Ocean  ").expect("valid"), "Ocean");
    assert!(validate_theme_name("   ").is_err());
    assert!(validate_theme_name("line\nbreak").is_err());
    assert!(validate_theme_name("brace{").is_err());
    assert!(validate_theme_name(&"x".repeat(MAX_THEME_NAME_LEN + 1)).is_err());
}

#[test]
fn export_file_names_are_slugs() {
    assert_eq!(
        suggested_file_name("Ocean Dusk"),
        "farcaster-theme-ocean-dusk.css"
    );
    assert_eq!(suggested_file_name("  "), "farcaster-theme.css");
    assert_eq!(
        suggested_file_name("Neon/Grid"),
        "farcaster-theme-neon-grid.css"
    );
}

#[test]
fn css_comments_are_ignored() {
    let css = format!("/* Ocean */\n{}", css("Ocean"));
    assert_eq!(
        ThemeDefinition::from_css(&css).expect("decode theme").name,
        "Ocean"
    );
}
