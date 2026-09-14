use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{Arc, Weak},
};

use gpui::{Image, ImageFormat};

use crate::conversation::EncodedImage;

// Weak owners preserve allocation identity without retaining conversation data.
struct CachedImage {
    encoded: Weak<EncodedImage>,
    image: Arc<Image>,
}

#[derive(Default)]
struct ImageCache {
    entries: VecDeque<CachedImage>,
}

const MAX_CACHED_IMAGES: usize = 128;

impl ImageCache {
    fn get_or_insert_with(
        &mut self,
        encoded: &Arc<EncodedImage>,
        create: impl FnOnce() -> Arc<Image>,
    ) -> Arc<Image> {
        let image = self
            .entries
            .iter()
            .position(|cached| cached.encoded.as_ptr() == Arc::as_ptr(encoded))
            .and_then(|index| self.entries.remove(index))
            .map(|cached| cached.image)
            .unwrap_or_else(create);
        self.entries
            .retain(|cached| cached.encoded.strong_count() > 0);
        if self.entries.len() == MAX_CACHED_IMAGES {
            self.entries.pop_front();
        }
        self.entries.push_back(CachedImage {
            encoded: Arc::downgrade(encoded),
            image: image.clone(),
        });
        image
    }
}

thread_local! {
    static CACHE: RefCell<ImageCache> = RefCell::new(ImageCache::default());
}

pub(crate) fn image(encoded: &Arc<EncodedImage>) -> Arc<Image> {
    CACHE.with_borrow_mut(|cache| {
        cache.get_or_insert_with(encoded, || {
            let format = ImageFormat::from_mime_type(encoded.mime_type())
                .expect("encoded transcript image MIME must be supported");
            Arc::new(Image::from_bytes(format, encoded.bytes().to_vec()))
        })
    })
}

pub(crate) fn from_preview(preview: Arc<Image>) -> Option<Arc<EncodedImage>> {
    let encoded = Arc::new(EncodedImage::new(
        preview.bytes().to_vec(),
        preview.format().mime_type(),
    )?);
    CACHE.with_borrow_mut(|cache| cache.get_or_insert_with(&encoded, || preview));
    Some(encoded)
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
