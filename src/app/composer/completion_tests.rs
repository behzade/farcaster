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
fn standalone_skills_submit_but_backend_commands_only_complete() {
    let skill = command("skill:review", SlashCommandSource::Skill);
    let skill_completion =
        resolve("$rev", 4, &[], 0, std::slice::from_ref(&skill)).expect("skill completion");
    assert_eq!(skill_completion.snapshot.text, "$review ");
    assert!(skill_completion.submit);

    let reload = command("reload", SlashCommandSource::Extension);
    let slash_completion = resolve("/rel", 4, &[], 0, &[reload]).expect("slash completion");
    assert_eq!(slash_completion.snapshot.text, "/reload ");
    assert!(!slash_completion.submit);
}

#[test]
fn selection_completes_the_highlighted_backend_command() {
    let commands = [
        command("review", SlashCommandSource::Prompt),
        command("reload", SlashCommandSource::Extension),
    ];
    let completion = resolve("/r", 2, &[], 1, &commands).expect("selected completion");

    assert_eq!(completion.snapshot.text, "/reload ");
    assert!(!completion.submit);
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
    let slash_commands = [
        command("review", SlashCommandSource::Prompt),
        command("reload", SlashCommandSource::Extension),
        command("model", SlashCommandSource::Prompt),
    ];
    assert!(
        !resolve("/r", 2, &[], 0, &slash_commands)
            .expect("ambiguous completion")
            .submit
    );
    assert!(
        !resolve("/mod", 4, &[], 0, &slash_commands)
            .expect("backend command completion")
            .submit
    );
    assert!(resolve("$review ", 8, &[], 0, std::slice::from_ref(&skill)).is_none());
    assert!(resolve("/reload ", 8, &[], 0, &slash_commands).is_none());
}

#[test]
fn enter_has_no_completion_for_regular_prompt_text() {
    assert!(resolve("send this", 9, &[], 0, &[]).is_none());
}
