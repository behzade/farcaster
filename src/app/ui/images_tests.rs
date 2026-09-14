use std::sync::Arc;

use gpui::{Image, ImageFormat};

use super::{ImageCache, MAX_CACHED_IMAGES, from_preview, image};
use crate::conversation::EncodedImage;

#[test]
fn composer_preview_round_trip_preserves_bytes_and_reuses_the_image() {
    let preview = Arc::new(Image::from_bytes(ImageFormat::Gif, vec![4, 5, 6]));
    let encoded = from_preview(preview.clone()).unwrap();
    assert_eq!(encoded.mime_type(), "image/gif");
    assert_eq!(encoded.bytes(), preview.bytes());
    assert!(Arc::ptr_eq(&image(&encoded), &preview));
}

#[test]
fn cache_keeps_recent_images_and_releases_evicted_or_unowned_images() {
    let mut cache = ImageCache::default();
    let encoded = (0..=MAX_CACHED_IMAGES)
        .map(|index| {
            Arc::new(EncodedImage::new(index.to_le_bytes().to_vec(), "image/png").unwrap())
        })
        .collect::<Vec<_>>();
    let create = || Arc::new(Image::from_bytes(ImageFormat::Png, vec![1]));
    let first = cache.get_or_insert_with(&encoded[0], create);
    let second = Arc::downgrade(&cache.get_or_insert_with(&encoded[1], create));
    for item in &encoded[2..MAX_CACHED_IMAGES] {
        cache.get_or_insert_with(item, create);
    }
    assert!(Arc::ptr_eq(
        &cache.get_or_insert_with(&encoded[0], || panic!("cache miss")),
        &first
    ));
    cache.get_or_insert_with(&encoded[MAX_CACHED_IMAGES], create);
    assert!(second.upgrade().is_none());
    assert_eq!(cache.entries.len(), MAX_CACHED_IMAGES);

    let last = encoded.last().unwrap().clone();
    let released = Arc::downgrade(&first);
    drop(first);
    drop(encoded);
    cache.get_or_insert_with(&last, || panic!("cache miss"));
    assert!(released.upgrade().is_none());
    assert_eq!(cache.entries.len(), 1);
}
