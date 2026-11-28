use super::*;
use crate::agents::Backend;
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
            Backend::Pi,
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

#[gpui::test]
fn removing_an_attached_text_file_preserves_the_original(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::removing_an_attached_text_file_preserves_the_original"
        ),
        cx,
        |cx, app, _, project| {
            let original = project.join("example.rs");
            std::fs::write(&original, "let value = 1;\n").unwrap();
            let LoadedAttachment::Text(paste) = load_attachment(original.clone()).unwrap() else {
                panic!("text attachment");
            };
            assert_ne!(paste.path, original);
            assert_eq!(paste.file_name(), "example.rs");
            let attached = paste.path.clone();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let target = app.composer.sessions.current_target().to_owned();
                    app.composer
                        .pastes
                        .entry(target.clone())
                        .or_default()
                        .push(paste);
                    app.save_composer_attachments(&target);
                    app.remove_composer_paste(0, cx);
                })
            });
            assert!(!attached.exists());
            assert_eq!(
                std::fs::read_to_string(original).unwrap(),
                "let value = 1;\n"
            );
        },
    );
}
