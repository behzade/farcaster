use super::*;
use crate::{
    app::infrastructure::persistence::{ComposerRecord, StateStore},
    protocol::{PromptImage, PromptMode},
};

#[test]
fn unsent_images_and_text_files_survive_database_reopen() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let path = temp.path().join("paste.txt");
    std::fs::write(&path, "first line\nsecond line")?;
    let target = "draft:attachments";
    {
        let store = StateStore::open_at(&database)?;
        store.enqueue_prompt(
            target,
            "pi",
            temp.path(),
            None,
            PromptMode::Normal,
            "setup",
            &[],
        )?;
        store.save_composer_session(&ComposerRecord {
            target: target.into(),
            attachments: vec![
                ComposerAttachment::Image(PromptImage::new("AQID".into(), "image/png".into())),
                ComposerAttachment::Image(PromptImage::new("BAUG".into(), "image/png".into())),
                ComposerAttachment::TextFile { path: path.clone() },
            ],
            ..ComposerRecord::default()
        })?;
    }
    let store = StateStore::open_at(&database)?;
    let records = store.load_composer_sessions()?;
    assert_eq!(records.len(), 1);
    assert!(records[0].text.is_empty());
    assert_eq!(records[0].attachments.len(), 3);
    let mut sessions = ComposerSessions::for_test(target.into());
    sessions.set_attachments(target, records[0].attachments.clone());
    let restored_target = "session:attachments";
    sessions.promote(target, restored_target.into());
    sessions.capture_current(super::super::sessions::ComposerSnapshot::new(
        "text".into(),
        4,
        4..4,
    ));
    let (images, pastes) = restore(&sessions);
    assert_eq!(images[restored_target].len(), 2);
    assert_eq!(images[restored_target][0].prompt.bytes()?, vec![1, 2, 3]);
    assert_eq!(images[restored_target][1].preview.bytes(), &[4, 5, 6]);
    assert!(
        images[restored_target]
            .iter()
            .all(|image| image.prompt.data.is_empty())
    );
    assert_eq!(
        pastes[restored_target][0].content,
        "first line\nsecond line"
    );
    assert_eq!(pastes[restored_target][0].path, path);
    assert_eq!(pastes[restored_target][0].line_count, 2);
    store.save_composer_session(&ComposerRecord {
        target: target.into(),
        ..ComposerRecord::default()
    })?;
    drop(store);
    assert!(
        StateStore::open_at(&database)?.load_composer_sessions()?[0]
            .attachments
            .is_empty()
    );
    Ok(())
}
