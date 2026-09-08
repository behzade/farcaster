use std::io::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{PromptImage, StateStore};

// Only the hash is persisted, so moving the database and its images together works.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum StoredImage {
    File {
        attachment: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Inline(PromptImage),
}

impl StateStore {
    pub(crate) fn store_prompt_images(
        &self,
        images: &[PromptImage],
    ) -> Result<Vec<PromptImage>, String> {
        images
            .iter()
            .map(|image| {
                Ok(PromptImage::from_file(
                    self.image_directory.join(self.store_image(image)?),
                    image.mime_type.clone(),
                ))
            })
            .collect()
    }

    pub(super) fn encode_prompt_images(&self, images: &[PromptImage]) -> Result<String, String> {
        let stored = images
            .iter()
            .map(|image| {
                Ok(StoredImage::File {
                    attachment: self.store_image(image)?,
                    mime_type: image.mime_type.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        serde_json::to_string(&stored).map_err(|error| format!("encode prompt images: {error}"))
    }

    fn store_image(&self, image: &PromptImage) -> Result<String, String> {
        let bytes = image.bytes()?;
        if bytes.is_empty() {
            return Err("Cannot save an empty image".into());
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let path = self.image_directory.join(&hash);
        std::fs::create_dir_all(&self.image_directory)
            .map_err(|error| format!("create image directory: {error}"))?;
        // Publish complete bytes atomically; simultaneous writers can share a hash.
        if !path.exists() {
            let mut file = tempfile::NamedTempFile::new_in(&self.image_directory)
                .map_err(|error| format!("create image: {error}"))?;
            file.write_all(&bytes)
                .and_then(|()| file.as_file().sync_all())
                .map_err(|error| format!("write image: {error}"))?;
            if let Err(error) = file.persist_noclobber(&path)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(format!("save image: {error}"));
            }
            std::fs::File::open(&self.image_directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|error| format!("sync image directory: {error}"))?;
        }
        // Detect corruption rather than silently reusing the wrong bytes.
        if std::fs::read(&path).map_err(|error| format!("read saved image: {error}"))? != bytes {
            return Err(format!("Saved image {} is corrupt", path.display()));
        }
        Ok(hash)
    }

    pub(super) fn decode_prompt_images(&self, json: &str) -> Result<Vec<PromptImage>, String> {
        let stored: Vec<StoredImage> =
            serde_json::from_str(json).map_err(|error| format!("decode prompt images: {error}"))?;
        stored
            .into_iter()
            .map(|image| match image {
                StoredImage::File {
                    attachment,
                    mime_type,
                } => {
                    if attachment.len() != 64
                        || !attachment
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        return Err("Invalid image attachment hash".into());
                    }
                    Ok(PromptImage::from_file(
                        self.image_directory.join(attachment),
                        mime_type,
                    ))
                }
                StoredImage::Inline(image) => Ok(image),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
