use std::cell::RefCell;

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};

use crate::{
    app::composer::prompt_fragments,
    protocol::{SlashCommand, SlashCommandSource},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerSuggestion {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) sigil: char,
}

thread_local! {
    static SUGGESTION_MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

pub(super) fn fuzzy_suggestions(
    suggestions: impl IntoIterator<Item = ComposerSuggestion>,
    query: &str,
) -> Vec<ComposerSuggestion> {
    if query.is_empty() {
        return suggestions.into_iter().collect();
    }
    let pattern = Atom::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );
    SUGGESTION_MATCHER.with(|matcher| {
        let mut matcher = matcher.borrow_mut();
        let mut buffer = Vec::new();
        let mut matches = suggestions
            .into_iter()
            .filter_map(|suggestion| {
                let score =
                    pattern.score(Utf32Str::new(&suggestion.name, &mut buffer), &mut matcher)?;
                Some((suggestion, score))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(suggestion, score)| {
            std::cmp::Reverse((suggestion.name.eq_ignore_ascii_case(query), *score))
        });
        matches
            .into_iter()
            .map(|(suggestion, _)| suggestion)
            .collect()
    })
}

pub(crate) fn contains_invocation(input: &str, commands: &[SlashCommand]) -> bool {
    recognized_invocations(input, commands).next().is_some()
}

pub(crate) fn recognized_invocations(
    input: &str,
    commands: &[SlashCommand],
) -> impl Iterator<Item = (std::ops::Range<usize>, SlashCommandSource)> {
    let invocable = invocable_commands(commands);
    let aliases = invocable
        .iter()
        .map(|command| (invocation_alias(command, &invocable), command.source))
        .collect::<Vec<_>>();
    prompt_fragments::tokens(input).filter_map(move |(start, _, token)| {
        let token = prompt_fragments::invocation_token(token);
        let name = token.strip_prefix('$')?;
        let (_, source) = aliases.iter().find(|(alias, _)| alias == name)?;
        Some((start..start + token.len(), *source))
    })
}

pub(crate) fn suggestions(input: &str, commands: &[SlashCommand]) -> Vec<ComposerSuggestion> {
    let invocable = invocable_commands(commands);
    let Some(query) = invocation_query(input) else {
        return Vec::new();
    };
    let mut ordered = invocable.iter().collect::<Vec<_>>();
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
    let suggestions = ordered.into_iter().map(|command| {
        let name = invocation_alias(command, &invocable);
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
    });
    fuzzy_suggestions(suggestions, query)
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
            character.is_alphabetic()
                || character.is_ascii_digit()
                || matches!(character, ':' | '_' | '-')
        })
        .then_some(name)
}

fn invocable_commands(commands: &[SlashCommand]) -> Vec<SlashCommand> {
    let owned = prompt_fragments::commands();
    let mut invocable = Vec::new();
    for command in owned.iter().chain(commands).filter(|command| {
        matches!(
            command.source,
            SlashCommandSource::Prompt | SlashCommandSource::Skill
        )
    }) {
        if !invocable.iter().any(|existing: &SlashCommand| {
            existing.source == command.source && existing.name == command.name
        }) {
            invocable.push(command.clone());
        }
    }
    invocable
}

fn invocation_alias(command: &SlashCommand, commands: &[SlashCommand]) -> String {
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
#[path = "user_invocations_tests.rs"]
mod tests;
