use super::*;

#[test]
fn catalog_exposes_compaction_alongside_native_commands() {
    let commands = catalog(json!({"data":[
        {"name":"init", "description":"Initialize"}, {"name":"review"}, {"name":"compact"}
    ]}))
    .expect("catalog");
    assert_eq!(
        commands
            .iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["compact", "init", "review"]
    );
    assert_eq!(commands[0]["source"], "extension");
    assert!(catalog(json!({"data": null})).is_err());
}

#[test]
fn invocations_preserve_arguments_and_leave_other_text_as_prompts() {
    let commands = ["review".to_owned(), "team/review".to_owned()].into();
    assert_eq!(invocation(" /compact  ", &commands), Some(("compact", "")));
    assert_eq!(
        invocation("/team/review\tsrc/lib.rs  tests", &commands),
        Some(("team/review", "src/lib.rs  tests"))
    );
    for text in [
        "review",
        "please /review",
        "/review-other",
        "/Users/me/file",
    ] {
        assert_eq!(invocation(text, &commands), None);
    }
}
