//! Slash-command matching against Pi's RPC command catalog.

use crate::protocol::{SlashCommand, SlashCommandSource};

pub(super) fn exact<'a>(input: &str, commands: &'a [SlashCommand]) -> Option<&'a SlashCommand> {
    let command_name = command_name(input)?;
    commands.iter().find(|command| command.name == command_name)
}

pub(super) fn suggestions<'a>(input: &str, commands: &'a [SlashCommand]) -> Vec<&'a SlashCommand> {
    let Some(query) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if query.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for command in commands {
        if command.name.starts_with(query)
            && !matches
                .iter()
                .any(|existing: &&SlashCommand| existing.name == command.name)
        {
            matches.push(command);
        }
    }
    matches
}

pub(super) fn is_immediate_extension(input: &str, commands: &[SlashCommand]) -> bool {
    exact(input, commands).is_some_and(|command| command.source == SlashCommandSource::Extension)
}

fn command_name(input: &str) -> Option<&str> {
    let command = input.strip_prefix('/')?;
    let name = command.split_once(' ').map_or(command, |(name, _)| name);
    (!name.is_empty()).then_some(name)
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
    fn execution_requires_an_exact_catalog_name() {
        let commands = vec![
            command("reload", SlashCommandSource::Extension),
            command("review-loop", SlashCommandSource::Prompt),
        ];

        assert_eq!(exact("/reload", &commands), Some(&commands[0]));
        assert_eq!(exact("/reload now", &commands), Some(&commands[0]));
        assert_eq!(exact("/review-loop", &commands), Some(&commands[1]));
        assert_eq!(exact("/re", &commands), None);
        assert_eq!(exact("/reload-more", &commands), None);
        assert_eq!(exact("/missing", &commands), None);
        assert_eq!(exact("explain /reload", &commands), None);
        assert_eq!(exact("/reload\nnow", &commands), None);
        assert!(is_immediate_extension("/reload", &commands));
        assert!(is_immediate_extension("/reload now", &commands));
        assert!(!is_immediate_extension("/re", &commands));
        assert!(!is_immediate_extension("/review-loop", &commands));
    }

    #[test]
    fn suggestions_use_prefixes_without_changing_execution_matching() {
        let commands = vec![
            command("reload", SlashCommandSource::Extension),
            command("review", SlashCommandSource::Prompt),
            command("reload", SlashCommandSource::Skill),
        ];

        assert_eq!(
            suggestions("/re", &commands)
                .into_iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["reload", "review"]
        );
        assert!(suggestions("/re now", &commands).is_empty());
        assert!(suggestions("not a command", &commands).is_empty());
    }
}
