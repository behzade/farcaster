//! Composer completion for extension-expanded user invocations.

use crate::protocol::{SlashCommand, SlashCommandSource};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerSuggestion {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) sigil: char,
}

pub(crate) fn suggestions(input: &str, commands: &[SlashCommand]) -> Vec<ComposerSuggestion> {
    let invocable = invocable_commands(commands);
    let Some(query) = invocation_query(input, &invocable) else {
        return Vec::new();
    };
    invocable
        .iter()
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

fn invocation_query<'a>(input: &'a str, commands: &[&SlashCommand]) -> Option<&'a str> {
    if input.ends_with(char::is_whitespace) {
        return None;
    }
    let mut tokens = input.split_whitespace().peekable();
    let mut query = None;
    while let Some(token) = tokens.next() {
        let name = token.strip_prefix('$')?;
        if name.chars().any(|character| {
            !(character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, ':' | '_' | '-'))
        }) {
            return None;
        }
        if tokens.peek().is_none() {
            query = Some(name);
        } else if name.is_empty()
            || !commands
                .iter()
                .any(|command| invocation_alias(command, commands) == name)
        {
            return None;
        }
    }
    query
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
        let suggestion = suggestions("$simplify $com", &commands)
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
        assert!(suggestions("explain $com", &commands).is_empty());
        assert!(suggestions("$missing $com", &commands).is_empty());
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
