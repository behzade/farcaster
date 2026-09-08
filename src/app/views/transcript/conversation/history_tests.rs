use super::*;

#[test]
fn file_and_inline_images_render_the_same_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("image");
    std::fs::write(&path, [1, 2, 3])?;
    let images = decode_prompt_images(&[
        PromptImage::from_file(path, "image/png".into()),
        PromptImage::new("AQID".into(), "image/png".into()),
    ]);
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].bytes(), images[1].bytes());
    Ok(())
}
