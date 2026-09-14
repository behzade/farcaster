use std::sync::Arc;

use crate::protocol::PromptImage;

/// Encoded image-file bytes, independent of the renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EncodedImage {
    mime_type: &'static str,
    bytes: Arc<[u8]>,
}

impl EncodedImage {
    pub(crate) fn new(bytes: Vec<u8>, mime_type: &str) -> Option<Self> {
        let mime_type = canonical_mime_type(mime_type)?;
        (!bytes.is_empty()).then(|| Self {
            mime_type,
            bytes: bytes.into(),
        })
    }

    pub(crate) fn from_prompt(image: &PromptImage) -> Option<Self> {
        let mime_type = canonical_mime_type(&image.mime_type)?;
        Self::new(image.bytes().ok()?, mime_type)
    }

    pub(crate) const fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn canonical_mime_type(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        "image/svg+xml" => Some("image/svg+xml"),
        "image/bmp" => Some("image/bmp"),
        "image/tiff" | "image/tif" => Some("image/tiff"),
        "image/ico" => Some("image/ico"),
        "image/x-portable-anymap" => Some("image/x-portable-anymap"),
        _ => None,
    }
}
