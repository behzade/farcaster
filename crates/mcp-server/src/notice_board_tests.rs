use super::*;

#[test]
fn snapshots_are_newest_first_and_posts_emit_updates() -> Result<(), String> {
    let board = NoticeBoard::default();
    let updates = board.updates();
    for (id, name) in [("one", "OrangeCoyote"), ("two", "SilverHeron")] {
        board.post(
            Path::new("/project"),
            id.into(),
            name.into(),
            "editing shared files".into(),
            vec!["src".into()],
        )?;
    }

    assert!(updates.try_recv().is_ok());
    assert_eq!(
        board
            .snapshot(Path::new("/project"))
            .into_iter()
            .map(|notice| notice.from)
            .collect::<Vec<_>>(),
        ["SilverHeron", "OrangeCoyote"]
    );
    Ok(())
}
