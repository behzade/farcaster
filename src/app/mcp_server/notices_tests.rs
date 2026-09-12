use super::*;

fn caller(id: &str, name: &str) -> CallerContext {
    CallerContext {
        worker_id: id.into(),
        worker_name: name.into(),
        project: "/project".into(),
        session: format!("session-{id}"),
        backend: "pi".into(),
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}

#[test]
fn post_returns_other_relevant_notices_without_internal_ids() -> Result<(), String> {
    let board = NoticeBoard::default();
    let first = caller("internal-1", "OrangeCoyote");
    let second = caller("internal-2", "SilverHeron");
    board.access(
        &first,
        Params {
            action: Action::Post,
            message: Some("editing parser".into()),
            paths: vec!["src/parser".into()],
        },
    )?;
    let response = board.access(
        &second,
        Params {
            action: Action::Post,
            message: Some("preparing parser commit".into()),
            paths: vec!["src/parser/mod.rs".into()],
        },
    )?;

    assert!(response.posted);
    assert_eq!(response.notices[0].from, "OrangeCoyote");
    assert_eq!(response.notices[0].message, "editing parser");
    assert!(
        !serde_json::to_string(&response)
            .map_err(|error| error.to_string())?
            .contains("internal-1")
    );
    Ok(())
}

#[test]
fn reads_can_filter_unrelated_paths() -> Result<(), String> {
    let board = NoticeBoard::default();
    let first = caller("one", "OrangeCoyote");
    let second = caller("two", "SilverHeron");
    board.access(
        &first,
        Params {
            action: Action::Post,
            message: Some("editing parser".into()),
            paths: vec!["src/parser.rs".into()],
        },
    )?;
    let response = board.access(
        &second,
        Params {
            action: Action::Read,
            message: None,
            paths: vec!["src/ui".into()],
        },
    )?;
    assert!(response.notices.is_empty());
    Ok(())
}

#[test]
fn children_cannot_access_the_board() {
    let board = NoticeBoard::default();
    let mut child = caller("child", "review");
    child.parent_worker_id = Some("parent".into());
    assert!(
        board
            .access(
                &child,
                Params {
                    action: Action::Read,
                    message: None,
                    paths: Vec::new(),
                },
            )
            .is_err()
    );
}
