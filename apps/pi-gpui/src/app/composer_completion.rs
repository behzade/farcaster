//! Enter-key completion policy for the prompt composer.

use crate::{composer_sessions::ComposerSnapshot, protocol::SlashCommand, user_invocations};

use super::{file_mentions, slash_commands};

pub(super) struct ComposerCompletion {
    pub(super) snapshot: ComposerSnapshot,
    pub(super) submit: bool,
}

pub(super) fn resolve(
    text: &str,
    cursor: usize,
    project_files: &[String],
    mention_selection: usize,
    commands: &[SlashCommand],
) -> Option<ComposerCompletion> {
    if let Some(query) = file_mentions::query_at_cursor(text, cursor) {
        let matches = file_mentions::matches(project_files, &query.text);
        if let Some(path) = matches.get(mention_selection.min(matches.len().saturating_sub(1))) {
            let (text, cursor) = file_mentions::insert(text, &query, path);
            return Some(ComposerCompletion {
                snapshot: ComposerSnapshot::new(text, cursor, cursor..cursor),
                submit: false,
            });
        }
    }

    let prefix = text.get(..cursor)?;
    let mut suggestions = slash_commands::suggestions(text.trim_start(), commands)
        .into_iter()
        .chain(user_invocations::suggestions(prefix, commands));
    let suggestion = suggestions.next()?;
    let standalone = cursor == text.len() && text.split_whitespace().count() == 1;
    let submit = standalone
        && suggestions.next().is_none()
        && (suggestion.sigil == '$' || slash_commands::submits_after_completion(&suggestion.name));
    let (text, cursor) =
        user_invocations::complete(text, cursor, suggestion.sigil, &suggestion.name);
    Some(ComposerCompletion {
        snapshot: ComposerSnapshot::new(text, cursor, cursor..cursor),
        submit,
    })
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

        assert_eq!(completion.snapshot.text, "read @src/main.rs ");
        assert_eq!(completion.snapshot.cursor, completion.snapshot.text.len());
        assert!(!completion.submit);
    }

    #[test]
    fn standalone_skills_and_argumentless_slash_commands_submit_after_completion() {
        let skill = command("skill:review", SlashCommandSource::Skill);
        let skill_completion =
            resolve("$rev", 4, &[], 0, std::slice::from_ref(&skill)).expect("skill completion");
        assert_eq!(skill_completion.snapshot.text, "$review ");
        assert!(skill_completion.submit);

        let slash_completion = resolve("/rel", 4, &[], 0, &[]).expect("slash completion");
        assert_eq!(slash_completion.snapshot.text, "/reload ");
        assert!(slash_completion.submit);
    }

    #[test]
    fn composed_ambiguous_and_argument_accepting_completions_do_not_submit() {
        let skill = command("skill:review", SlashCommandSource::Skill);
        assert!(
            !resolve(
                "please $rev",
                "please $rev".len(),
                &[],
                0,
                std::slice::from_ref(&skill),
            )
            .expect("composed skill completion")
            .submit
        );
        assert!(
            !resolve("/r", 2, &[], 0, &[])
                .expect("ambiguous completion")
                .submit
        );
        assert!(
            !resolve("/mod", 4, &[], 0, &[])
                .expect("argument-accepting command completion")
                .submit
        );
        assert!(resolve("$review ", 8, &[], 0, std::slice::from_ref(&skill)).is_none());
        assert!(resolve("/reload ", 8, &[], 0, &[]).is_none());
    }

    #[test]
    fn enter_has_no_completion_for_regular_prompt_text() {
        assert!(resolve("send this", 9, &[], 0, &[]).is_none());
    }
}
