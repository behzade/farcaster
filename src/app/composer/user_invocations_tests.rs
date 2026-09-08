use super::*;

fn command(name: &str, source: SlashCommandSource) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: None,
        source,
    }
}

#[test]
fn dollar_suggestions_compose_prompts_and_skills() {
    let commands = vec![
        command("skill:review", SlashCommandSource::Skill),
        command("reload", SlashCommandSource::Extension),
    ];

    assert_eq!(
        suggestions("$", &commands)
            .into_iter()
            .map(|suggestion| suggestion.name)
            .collect::<Vec<_>>(),
        ["commit", "simplify", "show-me", "review"]
    );
    let suggestion = suggestions("please $com", &commands)
        .into_iter()
        .next()
        .expect("commit suggestion");
    assert_eq!(suggestion.name, "commit");
    assert_eq!(
        complete(
            "$simplify $com later",
            "$simplify $com".len(),
            suggestion.sigil,
            &suggestion.name,
        ),
        ("$simplify $commit later".into(), "$simplify $commit ".len())
    );
    assert_eq!(suggestions("please $", &commands).len(), 4);
    assert!(suggestions("please$com", &commands).is_empty());
}

#[test]
fn invocation_detection_uses_the_command_catalog() {
    let commands = Vec::new();

    assert!(contains_invocation("please $simplify this", &commands));
    assert!(contains_invocation("$commit.", &commands));
    assert!(contains_invocation(
        "please $simplify, then $commit!",
        &commands
    ));
    assert!(!contains_invocation("$commit.md", &commands));
    assert!(!contains_invocation(r"\$commit.", &commands));
    assert!(!contains_invocation("cost $100", &commands));
    assert!(!contains_invocation("please $unknown", &commands));
}

#[test]
fn initial_suggestions_reserve_space_for_a_skill() {
    let mut commands = (0..8)
        .map(|index| command(&format!("prompt-{index}"), SlashCommandSource::Prompt))
        .collect::<Vec<_>>();
    commands.push(command("skill:review", SlashCommandSource::Skill));

    assert!(
        suggestions("$", &commands)
            .into_iter()
            .take(8)
            .any(|suggestion| suggestion.name == "review")
    );
}

#[test]
fn colliding_prompt_and_skill_names_are_source_qualified() {
    let commands = vec![
        command("review", SlashCommandSource::Prompt),
        command("review", SlashCommandSource::Prompt),
        command("skill:review", SlashCommandSource::Skill),
    ];
    assert_eq!(
        suggestions("$rev", &commands)
            .into_iter()
            .map(|suggestion| suggestion.name)
            .collect::<Vec<_>>(),
        ["prompt:review", "skill:review"]
    );
    assert!(!contains_invocation("$review", &commands));
    assert!(contains_invocation("$skill:review", &commands));
    assert!(contains_invocation("$prompt:review", &commands));
}
