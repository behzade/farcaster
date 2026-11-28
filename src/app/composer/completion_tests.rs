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
fn standalone_skills_and_backend_commands_only_insert_text() {
    let skill = command("skill:review", SlashCommandSource::Skill);
    let skill_completion =
        resolve("$rev", 4, &[], 0, std::slice::from_ref(&skill)).expect("skill completion");
    assert_eq!(skill_completion.text, "$review ");

    let reload = command("reload", SlashCommandSource::Extension);
    let slash_completion = resolve("/rel", 4, &[], 0, &[reload]).expect("slash completion");
    assert_eq!(slash_completion.text, "/reload ");
}

#[test]
fn selection_completes_the_highlighted_backend_command() {
    let commands = [
        command("review", SlashCommandSource::Prompt),
        command("reload", SlashCommandSource::Extension),
    ];
    let completion = resolve("/r", 2, &[], 1, &commands).expect("selected completion");

    assert_eq!(completion.text, "/reload ");
}

#[test]
fn completing_commands_and_skills_keeps_following_text_and_the_cursor() {
    let commands = [
        command("review", SlashCommandSource::Extension),
        command("skill:check", SlashCommandSource::Skill),
    ];
    let completion = resolve("/rev these changes", 4, &[], 0, &commands).unwrap();
    assert_eq!(completion.text, "/review these changes");
    assert_eq!(completion.cursor, 8);
    let completion = resolve("please $ch this", 10, &[], 0, &commands).unwrap();
    assert_eq!(completion.text, "please $check this");
    assert_eq!(completion.cursor, 14);
    assert!(resolve("/review ", 8, &[], 0, &commands).is_none());
    assert!(resolve("$check ", 7, &[], 0, &commands).is_none());
}

#[test]
fn enter_has_no_completion_for_regular_prompt_text() {
    assert!(resolve("send this", 9, &[], 0, &[]).is_none());
}
