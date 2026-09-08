use crate::{
    app::composer::{sessions::ComposerSnapshot, user_invocations},
    protocol::SlashCommand,
};

use super::{file_mentions, slash_commands};

pub(in crate::app) struct ComposerCompletion {
    pub(in crate::app) snapshot: ComposerSnapshot,
    pub(in crate::app) submit: bool,
}

#[cfg(test)]
pub(in crate::app) fn resolve(
    text: &str,
    cursor: usize,
    project_files: &[String],
    suggestion_selection: usize,
    commands: &[SlashCommand],
) -> Option<ComposerCompletion> {
    resolve_for_harness(
        text,
        cursor,
        project_files,
        suggestion_selection,
        commands,
        "pi",
    )
}

pub(in crate::app) fn resolve_for_harness(
    text: &str,
    cursor: usize,
    project_files: &[String],
    suggestion_selection: usize,
    commands: &[SlashCommand],
    harness: &str,
) -> Option<ComposerCompletion> {
    if let Some(query) = file_mentions::query_at_cursor(text, cursor) {
        let matches = file_mentions::matches(project_files, &query.text);
        if let Some(path) = matches.get(suggestion_selection.min(matches.len().saturating_sub(1))) {
            let (text, cursor) = file_mentions::insert(text, &query, path);
            return Some(ComposerCompletion {
                snapshot: ComposerSnapshot::new(text, cursor, cursor..cursor),
                submit: false,
            });
        }
    }

    let prefix = text.get(..cursor)?;
    let suggestions = slash_commands::suggestions_for_harness(text.trim_start(), commands, harness)
        .into_iter()
        .chain(user_invocations::suggestions(prefix, commands))
        .collect::<Vec<_>>();
    let suggestion =
        suggestions.get(suggestion_selection.min(suggestions.len().saturating_sub(1)))?;
    let standalone = cursor == text.len() && text.split_whitespace().count() == 1;
    let submit = standalone && suggestions.len() == 1 && suggestion.sigil == '$';
    let (text, cursor) =
        user_invocations::complete(text, cursor, suggestion.sigil, &suggestion.name);
    Some(ComposerCompletion {
        snapshot: ComposerSnapshot::new(text, cursor, cursor..cursor),
        submit,
    })
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
