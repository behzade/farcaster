use super::destinations::choices;
use super::*;
use crate::sessions::SessionSummary;
use std::path::Path;

#[test]
fn comment_preserves_unsaved_code_and_nested_fences() {
    let context = CodeContext {
        path: "/project/it's code.md".into(),
        cursor_line: 2,
        cursor_column: 1,
        anchor_line: 4,
        anchor_column: 3,
        mode: "v".into(),
        text: "```rust\nسلام\n```".into(),
        modified: true,
    };
    let prompt = context.prompt("  Explain this  ");
    assert!(prompt.starts_with("Explain this\n\nCode context: /project/it's code.md:2:1–4:3"));
    assert!(prompt.contains("buffer has unsaved edits"));
    assert!(prompt.contains("\n````\n```rust\nسلام\n```\n````"));
}

#[test]
fn destinations_keep_current_then_recent_chats_in_this_project() {
    let project = Path::new("/project");
    let session = |name: &str, project: &Path, archived, age| {
        SessionSummary::from_cached(
            name.into(),
            Path::new("/sessions").join(name),
            project.into(),
            name.into(),
            String::new(),
            String::new(),
            None,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(age),
            0,
            Default::default(),
            archived,
            false,
            name.into(),
        )
    };
    let current = session("current", project, false, 1);
    let destination = CodeDestination {
        target: "draft:current".into(),
        session: Some(current.target()),
        label: "Current chat".into(),
        harness: current.harness.clone(),
    };
    let recent = session("recent", project, false, 5);
    let sessions = vec![
        current,
        session("old", project, false, 2),
        recent.clone(),
        recent,
        session("archived", project, true, 8),
        session("other", Path::new("/elsewhere"), false, 9),
    ];
    let choices = choices(project, destination, &sessions);
    assert_eq!(
        choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>(),
        ["Current chat", "recent", "old"]
    );
}
