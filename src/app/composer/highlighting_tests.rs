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
