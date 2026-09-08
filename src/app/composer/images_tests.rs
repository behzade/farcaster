use gpui::{ClipboardItem, Image, ImageFormat};

use super::composer_images;

#[test]
fn clipboard_images_become_prompt_attachments() {
    let clipboard = ClipboardItem::new_image(&Image {
        format: ImageFormat::Png,
        bytes: vec![1, 2, 3],
        id: 7,
    });

    let images = composer_images(&clipboard);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].prompt.mime_type, "image/png");
    assert_eq!(images[0].prompt.data, "AQID");
    assert_eq!(images[0].byte_len, 3);
}
