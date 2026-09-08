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
#[path = "slash_commands_tests.rs"]
mod tests;
