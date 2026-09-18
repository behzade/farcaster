use gpui::Rgba;
use gpui_component::theme::ThemeMode;

use super::{ColorKey, Colors, SyntaxKey, ThemeToken, builtin};
use crate::app::ui::file_icons;

pub(crate) const MAX_THEME_NAME_LEN: usize = 48;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Appearance {
    #[default]
    Dark,
    Light,
}

impl Appearance {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }

    pub(crate) fn mode(self) -> ThemeMode {
        match self {
            Appearance::Dark => ThemeMode::Dark,
            Appearance::Light => ThemeMode::Light,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ThemeDefinition {
    pub(crate) name: String,
    pub(crate) appearance: Appearance,
    pub(crate) colors: Colors,
    pub(crate) tokens: Vec<(ThemeToken, Rgba)>,
}

impl ThemeDefinition {
    pub(crate) fn color(&self, token: ThemeToken) -> Rgba {
        if let ThemeToken::Palette(key) = token {
            return self.colors.get(key);
        }
        self.overridden(token)
            .unwrap_or_else(|| default_color(self.colors, token))
    }

    pub(crate) fn is_custom(&self, token: ThemeToken) -> bool {
        self.overridden(token).is_some()
    }

    pub(crate) fn set_color(&mut self, token: ThemeToken, color: Rgba) {
        if let ThemeToken::Palette(key) = token {
            self.colors.set(key, color);
            return;
        }
        match self
            .tokens
            .iter_mut()
            .find(|(existing, _)| *existing == token)
        {
            Some(existing) => existing.1 = color,
            None => self.tokens.push((token, color)),
        }
    }

    fn overridden(&self, token: ThemeToken) -> Option<Rgba> {
        self.tokens
            .iter()
            .find(|(existing, _)| *existing == token)
            .map(|(_, color)| *color)
    }

    pub(crate) fn from_css(css: &str) -> Result<Self, String> {
        let mut themes = themes_from_css(css)?;
        match themes.len() {
            1 => Ok(themes.remove(0)),
            0 => Err("This file does not define a Farcaster theme.".to_owned()),
            _ => Err("This file defines more than one theme.".to_owned()),
        }
    }

    pub(crate) fn to_css(&self) -> Result<String, String> {
        self.validate()?;
        Ok(theme_block(self))
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_theme_name(&self.name).map(|_| ())
    }
}

pub(crate) fn token_name(token: ThemeToken) -> String {
    match token {
        ThemeToken::Palette(key) => key.name().to_owned(),
        ThemeToken::Syntax(key) => key.name().to_owned(),
        ThemeToken::Icon(index) => file_icons::token_name(index).unwrap_or_default(),
    }
}

pub(crate) fn token_label(token: ThemeToken) -> String {
    match token {
        ThemeToken::Palette(key) => key.label(),
        ThemeToken::Syntax(key) => key.label(),
        ThemeToken::Icon(index) => file_icons::ICON_NAMES
            .get(index)
            .copied()
            .unwrap_or_default()
            .to_owned(),
    }
}

fn default_color(colors: Colors, token: ThemeToken) -> Rgba {
    match token {
        ThemeToken::Palette(key) => colors.get(key),
        ThemeToken::Syntax(key) => key.default_color(colors),
        ThemeToken::Icon(index) => file_icons::native_color(index),
    }
}

fn token_from_name(name: &str) -> Option<ThemeToken> {
    if let Some(key) = SyntaxKey::from_name(name) {
        return Some(ThemeToken::Syntax(key));
    }
    let icon = name.strip_prefix("icon-")?;
    file_icons::ICON_NAMES
        .iter()
        .position(|candidate| *candidate == icon)
        .map(ThemeToken::Icon)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ThemeLibrary {
    selected: String,
    themes: Vec<ThemeDefinition>,
}

impl Default for ThemeLibrary {
    fn default() -> Self {
        Self {
            selected: default_selected(),
            themes: Vec::new(),
        }
    }
}

fn default_selected() -> String {
    builtin::default_definition().name.clone()
}

impl ThemeLibrary {
    pub(crate) fn from_css(css: &str, selected: Option<&str>) -> Result<Self, String> {
        let themes = themes_from_css(css)?;
        Ok(Self {
            selected: selected.unwrap_or_default().to_owned(),
            themes,
        }
        .normalized())
    }

    pub(crate) fn to_css(&self) -> String {
        self.themes
            .iter()
            .map(theme_block)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn normalized(mut self) -> Self {
        let mut themes = Vec::with_capacity(self.themes.len());
        for mut theme in self.themes.drain(..) {
            let Ok(name) = validate_theme_name(&theme.name) else {
                continue;
            };
            let taken = builtin::is_built_in(&name)
                || themes
                    .iter()
                    .any(|kept: &ThemeDefinition| kept.name == name);
            if taken {
                continue;
            }
            theme.name = name;
            themes.push(theme);
        }
        self.themes = themes;
        if self.find(&self.selected).is_none() {
            self.selected = default_selected();
        }
        self
    }

    pub(crate) fn built_ins(&self) -> &'static [ThemeDefinition] {
        &*builtin::BUILT_IN_THEMES
    }

    pub(crate) fn user_themes(&self) -> &[ThemeDefinition] {
        &self.themes
    }

    pub(crate) fn display_order(&self) -> Vec<&ThemeDefinition> {
        self.built_ins().iter().chain(self.themes.iter()).collect()
    }

    pub(crate) fn selected_name(&self) -> &str {
        &self.selected
    }

    pub(crate) fn selected(&self) -> ThemeDefinition {
        self.find(&self.selected)
            .unwrap_or_else(|| builtin::default_definition().clone())
    }

    pub(crate) fn find(&self, name: &str) -> Option<ThemeDefinition> {
        self.built_ins()
            .iter()
            .chain(self.themes.iter())
            .find(|theme| theme.name == name)
            .cloned()
    }

    pub(crate) fn is_user_theme(&self, name: &str) -> bool {
        self.themes.iter().any(|theme| theme.name == name)
    }

    pub(crate) fn select(&mut self, name: &str) -> Result<(), String> {
        if self.find(name).is_none() {
            return Err(format!("Unknown theme: {name}"));
        }
        self.selected = name.to_owned();
        Ok(())
    }

    pub(crate) fn upsert(&mut self, definition: ThemeDefinition) -> Result<(), String> {
        definition.validate()?;
        if builtin::is_built_in(&definition.name) {
            return Err(format!(
                "{} ships with Farcaster. Rename the theme to save it.",
                definition.name
            ));
        }
        match self
            .themes
            .iter_mut()
            .find(|theme| theme.name == definition.name)
        {
            Some(existing) => *existing = definition,
            None => self.themes.push(definition),
        }
        Ok(())
    }

    pub(crate) fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        let to = validate_theme_name(to)?;
        let index = self
            .themes
            .iter()
            .position(|theme| theme.name == from)
            .ok_or_else(|| format!("{from} is not an editable theme."))?;
        if to != from && self.find(&to).is_some() {
            return Err(format!("A theme named {to} already exists."));
        }
        self.themes[index].name = to.clone();
        if self.selected == from {
            self.selected = to;
        }
        Ok(())
    }

    pub(crate) fn remove(&mut self, name: &str) -> Result<(), String> {
        let index = self
            .themes
            .iter()
            .position(|theme| theme.name == name)
            .ok_or_else(|| format!("{name} is a built-in theme and cannot be deleted."))?;
        self.themes.remove(index);
        if self.selected == name {
            self.selected = default_selected();
        }
        Ok(())
    }

    pub(crate) fn import(&mut self, css: &str) -> Result<Vec<String>, String> {
        let imported = themes_from_css(css)?;
        if imported.is_empty() {
            return Err("This file does not define a Farcaster theme.".to_owned());
        }
        let mut names = Vec::with_capacity(imported.len());
        for definition in imported {
            let name = self.unique_name(&definition.name);
            self.upsert(ThemeDefinition {
                name: name.clone(),
                ..definition
            })?;
            names.push(name);
        }
        Ok(names)
    }

    pub(crate) fn export(&self, name: &str) -> Result<String, String> {
        self.find(name)
            .ok_or_else(|| format!("Unknown theme: {name}"))?
            .to_css()
    }

    pub(crate) fn unique_name(&self, desired: &str) -> String {
        let base = validate_theme_name(desired).unwrap_or_else(|_| "Imported theme".to_owned());
        if self.find(&base).is_none() {
            return base;
        }
        for suffix in 2..1000 {
            let candidate = format!("{base} {suffix}");
            if self.find(&candidate).is_none() {
                return candidate;
            }
        }
        base
    }
}

pub(crate) fn validate_theme_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the theme a name before saving it.".to_owned());
    }
    if name.chars().count() > MAX_THEME_NAME_LEN {
        return Err(format!(
            "Theme names are limited to {MAX_THEME_NAME_LEN} characters."
        ));
    }
    if name
        .chars()
        .any(|character| character.is_control() || matches!(character, '"' | '{' | '}' | ';' | ':'))
    {
        return Err("Theme names cannot contain quotes or braces.".to_owned());
    }
    Ok(name.to_owned())
}

pub(crate) fn suggested_file_name(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "farcaster-theme.css".to_owned()
    } else {
        format!("farcaster-theme-{slug}.css")
    }
}

pub(crate) fn color_hex(color: Rgba) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b, a) = (
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    );
    if a == 0xff {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub(crate) fn parse_hex(value: &str) -> Option<Rgba> {
    Rgba::try_from(value.trim()).ok()
}

fn theme_block(definition: &ThemeDefinition) -> String {
    let mut css = format!(
        ":root[data-theme=\"{}\"][data-appearance=\"{}\"] {{\n",
        definition.name,
        definition.appearance.name()
    );
    for key in ColorKey::ALL {
        css.push_str(&format!(
            "  --{}: {};\n",
            key.name(),
            color_hex(definition.colors.get(*key))
        ));
    }
    for (token, color) in ordered_tokens(definition) {
        css.push_str(&format!(
            "  --{}: {};\n",
            token_name(token),
            color_hex(color)
        ));
    }
    css.push_str("}\n");
    css
}

fn ordered_tokens(definition: &ThemeDefinition) -> Vec<(ThemeToken, Rgba)> {
    let mut tokens: Vec<_> = definition.tokens.clone();
    tokens.sort_by_key(|(token, _)| match token {
        ThemeToken::Palette(key) => (0usize, ordinal_in(ColorKey::ALL, key)),
        ThemeToken::Icon(index) => (1, *index),
        ThemeToken::Syntax(key) => (2, ordinal_in(SyntaxKey::ALL, key)),
    });
    tokens
}

fn ordinal_in<T: PartialEq>(values: &[T], value: &T) -> usize {
    values
        .iter()
        .position(|candidate| candidate == value)
        .unwrap_or_default()
}

fn themes_from_css(css: &str) -> Result<Vec<ThemeDefinition>, String> {
    let css = strip_comments(css);
    let mut themes = Vec::new();
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().to_owned();
        let Some(close) = rest[open..].find('}') else {
            return Err("A theme block is missing its closing brace.".to_owned());
        };
        let body = rest[open + 1..open + close].to_owned();
        rest = &rest[open + close + 1..];
        if selector.is_empty() {
            continue;
        }
        themes.push(theme_from_block(&selector, &body)?);
    }
    Ok(themes)
}

fn theme_from_block(selector: &str, body: &str) -> Result<ThemeDefinition, String> {
    let name = attribute(selector, "data-theme")
        .ok_or_else(|| "Every theme block needs data-theme=\"Name\".".to_owned())?;
    let name = validate_theme_name(&name)?;
    let appearance = match attribute(selector, "data-appearance").as_deref() {
        None | Some("dark") => Appearance::Dark,
        Some("light") => Appearance::Light,
        Some(other) => {
            return Err(format!(
                "{name} has an unknown data-appearance: {other}. Use dark or light."
            ));
        }
    };
    let mut colors: Option<Colors> = None;
    let mut tokens: Vec<(ThemeToken, Rgba)> = Vec::new();
    let mut missing = ColorKey::ALL.to_vec();
    for declaration in body.split(';') {
        let Some((key, value)) = declaration.split_once(':') else {
            if declaration.trim().is_empty() {
                continue;
            }
            return Err(format!("{name} has a declaration without a value."));
        };
        let key = key.trim().strip_prefix("--").ok_or_else(|| {
            format!(
                "{name} uses {}. Only --color-token declarations are read.",
                key.trim()
            )
        })?;
        let value = value.trim();
        if let Some(color_key) = ColorKey::from_name(key) {
            let Some(color) = parse_hex(value) else {
                return Err(format!(
                    "{name} sets --{key} to {value}, which is not a color."
                ));
            };
            colors.get_or_insert(Colors::EMPTY).set(color_key, color);
            missing.retain(|candidate| *candidate != color_key);
            continue;
        }
        let Some(token) = token_from_name(key) else {
            return Err(format!("{name} sets an unknown token: --{key}"));
        };
        let Some(color) = parse_hex(value) else {
            return Err(format!(
                "{name} sets --{key} to {value}, which is not a color."
            ));
        };
        match tokens.iter_mut().find(|(existing, _)| *existing == token) {
            Some(existing) => existing.1 = color,
            None => tokens.push((token, color)),
        }
    }
    let Some(colors) = colors else {
        return Err(format!("{name} does not set any colors."));
    };
    if !missing.is_empty() {
        let missing = missing
            .iter()
            .map(|key| format!("--{}", key.name()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("{name} is missing: {missing}"));
    }
    Ok(ThemeDefinition {
        name,
        appearance,
        colors,
        tokens,
    })
}

fn attribute(selector: &str, name: &str) -> Option<String> {
    let start = selector.find(&format!("{name}="))? + name.len() + 1;
    let rest = selector[start..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

fn strip_comments(css: &str) -> String {
    let mut stripped = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return stripped,
        }
    }
    stripped.push_str(rest);
    stripped
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
