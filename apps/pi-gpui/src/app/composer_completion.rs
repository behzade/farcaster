//! Enter-key completion policy for the prompt composer.

use crate::{composer_sessions::ComposerSnapshot, protocol::SlashCommand, user_invocations};

use super::{file_mentions, slash_commands};

pub(super) fn resolve(
    text: &str,
    cursor: usize,
    project_files: &[String],
    mention_selection: usize,
    commands: &[SlashCommand],
) -> Option<ComposerSnapshot> {
    if let Some(query) = file_mentions::query_at_cursor(text, cursor) {
        let matches = file_mentions::matches(project_files, &query.text);
        if let Some(path) = matches.get(mention_selection.min(matches.len().saturating_sub(1))) {
            let (text, cursor) = file_mentions::insert(text, &query, path);
            return Some(ComposerSnapshot::new(text, cursor, cursor..cursor));
        }
    }

    let prefix = text.get(..cursor)?;
    let suggestion = slash_commands::suggestions(text.trim_start(), commands)
        .into_iter()
        .chain(user_invocations::suggestions(prefix, commands))
        .next()?;
    let (text, cursor) =
        user_invocations::complete(text, cursor, suggestion.sigil, &suggestion.name);
    Some(ComposerSnapshot::new(text, cursor, cursor..cursor))
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
    fn enter_completes_file_mentions_without_submitting_the_result() {
        let completion = resolve(
            "read @ma",
            "read @ma".len(),
            &["src/main.rs".into()],
            0,
            &[],
        )
        .expect("file completion");

        assert_eq!(completion.text, "read @src/main.rs ");
        assert_eq!(completion.cursor, completion.text.len());
    }

    #[test]
    fn enter_completes_skills_and_slash_commands_before_submission() {
        let skill = command("skill:review", SlashCommandSource::Skill);
        assert_eq!(
            resolve("$rev", 4, &[], 0, std::slice::from_ref(&skill))
                .expect("skill completion")
                .text,
            "$review "
        );
        assert_eq!(
            resolve("/rel", 4, &[], 0, &[])
                .expect("slash completion")
                .text,
            "/reload "
        );
        assert_eq!(
            resolve("$review ", 8, &[], 0, std::slice::from_ref(&skill)),
            None
        );
        assert_eq!(resolve("/reload ", 8, &[], 0, &[]), None);
    }

    #[test]
    fn enter_has_no_completion_for_regular_prompt_text() {
        assert_eq!(resolve("send this", 9, &[], 0, &[]), None);
    }
}
