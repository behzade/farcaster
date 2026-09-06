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
mod tests {
    use super::*;

    fn skill(name: &str) -> SlashCommand {
        SlashCommand {
            name: format!("skill:{name}"),
            description: None,
            source: SlashCommandSource::Skill,
        }
    }

    #[test]
    fn recognizes_complete_tokens_with_unicode_offsets_and_distinct_colors() {
        let text = "سلام $review,\n@src/main.rs $commit!";
        let spans = decorations(text, &[skill("review")], &["src/main.rs".into()]);
        assert_eq!(
            spans
                .iter()
                .map(|span| &text[span.range.clone()])
                .collect::<Vec<_>>(),
            ["$review", "@src/main.rs", "$commit"]
        );
        assert_eq!(spans[0].style.color, Some(THEME.colors.skill.into()));
        assert_eq!(spans[1].style.color, Some(THEME.colors.file.into()));
        assert_eq!(spans[2].style.color, Some(THEME.colors.accent.into()));
    }

    #[test]
    fn ignores_partial_unknown_escaped_and_embedded_tokens() {
        assert!(
            decorations(
                r"$rev $unknown \$review word$review $review.md @src/ @missing word@src/main.rs",
                &[skill("review")],
                &["src/main.rs".into()]
            )
            .is_empty()
        );
        assert!(decorations("$review", &[], &[]).is_empty());
    }
}
