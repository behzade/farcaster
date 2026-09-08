use gpui::HighlightStyle;
use gpui_base::input::TextDecoration;

use super::{prompt_fragments, user_invocations};
use crate::{
    app::ui::theme::THEME,
    protocol::{SlashCommand, SlashCommandSource},
};

pub(crate) fn decorations(
    text: &str,
    commands: &[SlashCommand],
    files: &[String],
) -> Vec<TextDecoration> {
    let invocations =
        user_invocations::recognized_invocations(text, commands).map(|(range, source)| {
            let color = if source == SlashCommandSource::Skill {
                THEME.colors.skill
            } else {
                THEME.colors.accent
            };
            (range, color)
        });
    let mentions = prompt_fragments::tokens(text).filter_map(|(start, end, token)| {
        let path = token.strip_prefix('@')?;
        files
            .iter()
            .any(|file| file == path)
            .then_some((start..end, THEME.colors.file))
    });
    let mut result = invocations
        .chain(mentions)
        .map(|(range, color)| {
            TextDecoration::new(
                range,
                HighlightStyle {
                    color: Some(color.into()),
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|decoration| decoration.range.start);
    result
}

#[cfg(test)]
#[path = "highlighting_tests.rs"]
mod tests;
