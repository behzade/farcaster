use crate::{app::composer::user_invocations::ComposerSuggestion, protocol::SlashCommand};

pub(in crate::app) fn exact<'a>(
    input: &str,
    commands: &'a [SlashCommand],
) -> Option<&'a SlashCommand> {
    let command_name = command_name(input)?;
    commands.iter().find(|command| command.name == command_name)
}

pub(in crate::app) fn is_exact_for_harness(
    input: &str,
    commands: &[SlashCommand],
    _harness: &str,
) -> bool {
    exact(input, commands).is_some()
}

pub(in crate::app) fn suggestions_for_harness(
    input: &str,
    commands: &[SlashCommand],
    _harness: &str,
) -> Vec<ComposerSuggestion> {
    suggestions(input, commands)
}

fn suggestions(input: &str, commands: &[SlashCommand]) -> Vec<ComposerSuggestion> {
    let Some(query) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if query.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for command in commands
        .iter()
        .filter(|command| command.name.starts_with(query))
    {
        if matches
            .iter()
            .any(|existing: &ComposerSuggestion| existing.name == command.name)
        {
            continue;
        }
        matches.push(ComposerSuggestion {
            name: command.name.clone(),
            description: command.description.clone(),
            sigil: '/',
        });
    }
    matches
}

fn command_name(input: &str) -> Option<&str> {
    let command = input.strip_prefix('/')?;
    let name = command.split_once(' ').map_or(command, |(name, _)| name);
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SlashCommandSource;

    fn command(name: &str, source: SlashCommandSource) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: None,
            source,
        }
    }

    #[test]
    fn execution_requires_an_exact_backend_catalog_name() {
        let commands = vec![
            command("extension-reload", SlashCommandSource::Extension),
            command("review-loop", SlashCommandSource::Prompt),
        ];

        assert_eq!(exact("/extension-reload", &commands), Some(&commands[0]));
        assert_eq!(
            exact("/extension-reload now", &commands),
            Some(&commands[0])
        );
        assert_eq!(exact("/review-loop", &commands), Some(&commands[1]));
        assert_eq!(exact("/re", &commands), None);
        assert_eq!(exact("/reload-more", &commands), None);
        assert_eq!(exact("/missing", &commands), None);
        assert_eq!(exact("explain /reload", &commands), None);
        assert_eq!(exact("/extension-reload\nnow", &commands), None);
    }

    #[test]
    fn suggestions_only_contain_backend_advertised_commands() {
        let commands = vec![
            command("reload", SlashCommandSource::Extension),
            command("review", SlashCommandSource::Prompt),
            command("skill:search", SlashCommandSource::Skill),
        ];

        assert_eq!(
            suggestions("/re", &commands)
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["reload", "review"]
        );
        assert!(suggestions("/", &[]).is_empty());
        assert!(suggestions("/settings", &[]).is_empty());
        assert!(suggestions("/re now", &commands).is_empty());
        assert!(suggestions("not a command", &commands).is_empty());
    }
}
