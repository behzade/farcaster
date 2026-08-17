//! In-memory image attachments keyed to each session composer.

use std::sync::Arc;

use base64::Engine as _;
use gpui::{ClipboardEntry, ClipboardItem, Context, Image};

use super::PiApp;
use crate::protocol::PromptImage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerImage {
    pub(crate) prompt: PromptImage,
    pub(crate) preview: Arc<Image>,
    pub(crate) byte_len: usize,
}

impl PiApp {
    pub(crate) fn has_composer_images(&self) -> bool {
        self.composer_images
            .get(self.composer_sessions.current_target())
            .is_some_and(|images| !images.is_empty())
    }

    pub(crate) fn current_composer_images(&self) -> &[ComposerImage] {
        self.composer_images
            .get(self.composer_sessions.current_target())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn paste_composer_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return false;
        };
        let images = composer_images(&clipboard);
        if images.is_empty() {
            return false;
        }
        let target = self.composer_sessions.current_target().to_owned();
        self.composer_images
            .entry(target)
            .or_default()
            .extend(images);
        self.notify_composer(cx);
        true
    }

    pub(crate) fn remove_composer_image(&mut self, index: usize, cx: &mut Context<Self>) {
        let target = self.composer_sessions.current_target().to_owned();
        if let Some(images) = self.composer_images.get_mut(&target)
            && index < images.len()
        {
            images.remove(index);
            if images.is_empty() {
                self.composer_images.remove(&target);
            }
            self.notify_composer(cx);
        }
    }
}

fn composer_images(clipboard: &ClipboardItem) -> Vec<ComposerImage> {
    clipboard
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::Image(image) if !image.bytes().is_empty() => Some(ComposerImage {
                prompt: PromptImage::new(
                    base64::engine::general_purpose::STANDARD.encode(image.bytes()),
                    image.format().mime_type().into(),
                ),
                preview: Arc::new(image.clone()),
                byte_len: image.bytes().len(),
            }),
            ClipboardEntry::String(_)
            | ClipboardEntry::ExternalPaths(_)
            | ClipboardEntry::Image(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
}
