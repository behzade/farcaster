use super::*;
use crate::protocol::PromptMode;

#[test]
fn queued_images_use_portable_deduplicated_files() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("original");
    let database = root.join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let image = PromptImage::new("aGVsbG8=".into(), "image/png".into());
    store.enqueue_prompt(
        "draft:images",
        "pi",
        directory.path(),
        None,
        PromptMode::Normal,
        "look",
        &[image.clone(), image.clone()],
    )?;
    let json: String = store
        .connection
        .query_row("SELECT images_json FROM outbox", [], |row| row.get(0))?;
    assert!(!json.contains("aGVsbG8="));
    assert!(!json.contains("original"));
    assert!(json.contains("attachment"));
    assert_eq!(std::fs::read_dir(root.join("images"))?.count(), 1);
    drop(store);

    let moved = directory.path().join("moved");
    std::fs::rename(&root, &moved)?;
    let store = StateStore::open_at(&moved.join("state.sqlite3"))?;
    let queued = store.queued_prompts()?;
    assert_eq!(queued[0].images.len(), 2);
    for attachment in &queued[0].images {
        assert!(attachment.data.is_empty());
        assert!(
            attachment
                .path
                .as_ref()
                .expect("test operation should succeed")
                .starts_with(&moved)
        );
        let wire = serde_json::to_value(attachment.clone().into_inline()?)?;
        assert_eq!(
            wire,
            serde_json::json!({"type":"image", "data":"aGVsbG8=", "mimeType":"image/png"})
        );
    }
    // Keep a missing attachment in the queue, and fail sending it explicitly.
    std::fs::remove_file(
        queued[0].images[0]
            .path
            .as_ref()
            .expect("test operation should succeed"),
    )?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    assert!(
        queued[0].images[0]
            .clone()
            .into_inline()
            .expect_err("invalid test input must fail")
            .contains("read image")
    );
    Ok(())
}

#[test]
fn legacy_images_load_and_invalid_references_fail() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = StateStore::open_at(&directory.path().join("state.sqlite3"))?;
    let image = PromptImage::new("AQID".into(), "image/png".into());
    assert_eq!(
        store.decode_prompt_images(&serde_json::to_string(std::slice::from_ref(&image))?)?,
        vec![image]
    );
    assert!(
        store
            .decode_prompt_images(r#"[{"attachment":"../secret", "mimeType":"image/png"}]"#)
            .is_err()
    );
    assert!(
        store
            .encode_prompt_images(&[PromptImage::new("not base64".into(), "image/png".into())])
            .is_err()
    );
    assert!(
        store
            .encode_prompt_images(&[PromptImage::new(String::new(), "image/png".into())])
            .is_err()
    );
    Ok(())
}

#[test]
fn corrupt_existing_image_is_not_reused() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = StateStore::open_at(&directory.path().join("state.sqlite3"))?;
    let image = PromptImage::new("AQID".into(), "image/png".into());
    let stored = store.store_prompt_images(std::slice::from_ref(&image))?;
    std::fs::write(
        stored[0]
            .path
            .as_ref()
            .expect("test operation should succeed"),
        b"wrong",
    )?;
    assert!(
        store
            .store_prompt_images(&[image])
            .expect_err("invalid test input must fail")
            .contains("corrupt")
    );
    Ok(())
}
