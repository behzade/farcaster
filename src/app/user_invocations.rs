use crate::protocol::{SlashCommand, SlashCommandSource};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerSuggestion {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) sigil: char,
}

pub(crate) fn contains_invocation(input: &str, commands: &[SlashCommand]) -> bool {
    let invocable = invocable_commands(commands);
    input.split_whitespace().any(|token| {
        token.strip_prefix('$').is_some_and(|name| {
            invocable
                .iter()
                .any(|command| invocation_alias(command, &invocable) == name)
        })
    })
}

pub(crate) fn suggestions(input: &str, commands: &[SlashCommand]) -> Vec<ComposerSuggestion> {
    let invocable = invocable_commands(commands);
    let Some(query) = invocation_query(input) else {
        return Vec::new();
    };
    let mut ordered = invocable.clone();
    if query.is_empty()
        && !ordered
            .iter()
            .take(8)
            .any(|command| command.source == SlashCommandSource::Skill)
        && let Some(index) = ordered
            .iter()
            .position(|command| command.source == SlashCommandSource::Skill)
    {
        let skill = ordered.remove(index);
        let index = 7.min(ordered.len());
        ordered.insert(index, skill);
    }
    ordered
        .into_iter()
        .filter_map(|command| {
            let name = invocation_alias(command, &invocable);
            name.contains(query).then(|| {
                let kind = if command.source == SlashCommandSource::Prompt {
                    "Prompt"
                } else {
                    "Skill"
                };
                ComposerSuggestion {
                    name,
                    description: Some(match &command.description {
                        Some(description) => format!("{kind} · {description}"),
                        None => kind.into(),
                    }),
                    sigil: '$',
                }
            })
        })
        .collect()
}

pub(crate) fn complete(input: &str, cursor: usize, sigil: char, name: &str) -> (String, usize) {
    if sigil == '/' {
        let text = format!("/{name} ");
        let cursor = text.len();
        return (text, cursor);
    }
    let token_start = input[..cursor]
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let replacement = format!("{sigil}{name} ");
    let suffix = input[cursor..]
        .strip_prefix(' ')
        .unwrap_or(&input[cursor..]);
    let text = format!("{}{}{}", &input[..token_start], replacement, suffix);
    let cursor = token_start + replacement.len();
    (text, cursor)
}

fn invocation_query(input: &str) -> Option<&str> {
    let token = input
        .rsplit_once(char::is_whitespace)
        .map_or(input, |(_, token)| token);
    let name = token.strip_prefix('$')?;
    name.chars()
        .all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, ':' | '_' | '-')
        })
        .then_some(name)
}

fn invocable_commands(commands: &[SlashCommand]) -> Vec<&SlashCommand> {
    let mut invocable = Vec::new();
    for command in commands.iter().filter(|command| {
        matches!(
            command.source,
            SlashCommandSource::Prompt | SlashCommandSource::Skill
        )
    }) {
        if !invocable.iter().any(|existing: &&SlashCommand| {
            existing.source == command.source && existing.name == command.name
        }) {
            invocable.push(command);
        }
    }
    invocable
}

fn invocation_alias(command: &SlashCommand, commands: &[&SlashCommand]) -> String {
    let bare_name = invocation_name(command);
    if commands
        .iter()
        .filter(|candidate| invocation_name(candidate) == bare_name)
        .count()
        == 1
    {
        bare_name.to_owned()
    } else {
        format!("{}:{bare_name}", invocation_source_name(command.source))
    }
}

fn invocation_name(command: &SlashCommand) -> &str {
    if command.source == SlashCommandSource::Skill {
        command.name.strip_prefix("skill:").unwrap_or(&command.name)
    } else {
        &command.name
    }
}

fn invocation_source_name(source: SlashCommandSource) -> &'static str {
    match source {
        SlashCommandSource::Prompt => "prompt",
        SlashCommandSource::Skill => "skill",
        SlashCommandSource::Extension => "extension",
    }
}

#[cfg(test)]
mod tests {
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
            command("simplify", SlashCommandSource::Prompt),
            command("skill:commit", SlashCommandSource::Skill),
            command("reload", SlashCommandSource::Extension),
        ];

        assert_eq!(
            suggestions("$", &commands)
                .into_iter()
                .map(|suggestion| suggestion.name)
                .collect::<Vec<_>>(),
            ["simplify", "commit"]
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
        assert_eq!(suggestions("please $", &commands).len(), 2);
        assert!(suggestions("please$com", &commands).is_empty());
    }

    #[test]
    fn invocation_detection_uses_the_command_catalog() {
        let commands = vec![command("simplify", SlashCommandSource::Prompt)];

        assert!(contains_invocation("please $simplify this", &commands));
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
            suggestions("$", &commands)
                .into_iter()
                .map(|suggestion| suggestion.name)
                .collect::<Vec<_>>(),
            ["prompt:review", "skill:review"]
        );
    }
}
