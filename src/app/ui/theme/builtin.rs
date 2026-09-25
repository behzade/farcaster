use std::sync::LazyLock;

use super::ThemeDefinition;

pub(crate) const THEME_FILES: [&str; 3] = [
    include_str!("../../../../assets/themes/farcaster.css"),
    include_str!("../../../../assets/themes/white.css"),
    include_str!("../../../../assets/themes/black.css"),
];

pub(crate) static BUILT_IN_THEMES: LazyLock<[ThemeDefinition; 3]> = LazyLock::new(|| {
    THEME_FILES.map(|file| ThemeDefinition::from_css(file).expect("bundled theme file"))
});

pub(crate) fn default_definition() -> &'static ThemeDefinition {
    &BUILT_IN_THEMES[0]
}

pub(crate) fn is_built_in(name: &str) -> bool {
    BUILT_IN_THEMES.iter().any(|theme| theme.name == name)
}
