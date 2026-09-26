use super::*;

fn chat(id: &str, target: &str) -> TaskChat {
    TaskChat {
        submission_id: id.into(),
        target: target.into(),
        project: PathBuf::from("/project"),
        session: None,
        new_task: false,
    }
}

#[test]
fn same_chat_submissions_keep_distinct_results() {
    let mut tasks = CodeTasks::default();
    tasks.track(chat("first", "session:one"));
    tasks.track(chat("second", "session:one"));
    tasks.associate("session:one", Some(std::path::Path::new("/session/one")));

    assert!(tasks.finish(None, "session:one").is_none());
    let second = tasks
        .finish(Some("second"), "session:one")
        .expect("second submission resolves");
    assert_eq!(second.submission_id, "second");
    assert_eq!(
        second.session.as_deref(),
        Some(std::path::Path::new("/session/one"))
    );
    let first = tasks
        .finish(Some("first"), "session:one")
        .expect("first submission resolves");
    assert_eq!(first.submission_id, "first");
    assert!(tasks.pending.is_empty());
}

#[test]
fn a_result_without_an_id_only_matches_one_pending_chat() {
    let mut tasks = CodeTasks::default();
    tasks.track(chat("only", "session:one"));
    assert_eq!(
        tasks
            .finish(None, "session:one")
            .expect("only submission resolves")
            .submission_id,
        "only"
    );
}
