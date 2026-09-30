use std::io::Read as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{PromptImage, StateStore};

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum StoredImage {
    File {
        attachment: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Inline(PromptImage),
}

impl StateStore {
    pub fn store_prompt_images(&self, images: &[PromptImage]) -> Result<Vec<PromptImage>, String> {
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
            .map(|image| self.encode_prompt_image(image))
            .collect::<Result<Vec<_>, String>>()?;
        serde_json::to_string(&stored).map_err(|error| format!("encode prompt images: {error}"))
    }

    pub(super) fn encode_prompt_image(&self, image: &PromptImage) -> Result<StoredImage, String> {
        Ok(StoredImage::File {
            attachment: self.store_image(image)?,
            mime_type: image.mime_type.clone(),
        })
    }

    fn store_image(&self, image: &PromptImage) -> Result<String, String> {
        let (hash, length) = read_image(image, &mut std::io::sink())?;
        if length == 0 {
            return Err("Cannot save an empty image".into());
        }
        let path = self.image_directory.join(&hash);
        if let Some(source) = &image.path
            && source.parent() == Some(self.image_directory.as_path())
            && source
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(valid_attachment_hash)
        {
            if source == &path {
                return Ok(hash);
            }
            return Err(format!("Saved image {} is corrupt", source.display()));
        }
        std::fs::create_dir_all(&self.image_directory)
            .map_err(|error| format!("create image directory: {error}"))?;
        if !path.exists() {
            let mut file = tempfile::NamedTempFile::new_in(&self.image_directory)
                .map_err(|error| format!("create image: {error}"))?;
            let (written_hash, _) = read_image(image, &mut file)?;
            if written_hash != hash {
                return Err("Image changed while saving".into());
            }
            file.as_file()
                .sync_all()
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
        let saved = PromptImage::from_file(path.clone(), image.mime_type.clone());
        if read_image(&saved, &mut std::io::sink())?.0 != hash {
            return Err(format!("Saved image {} is corrupt", path.display()));
        }
        Ok(hash)
    }

    pub(super) fn decode_prompt_images(&self, json: &str) -> Result<Vec<PromptImage>, String> {
        let stored: Vec<StoredImage> =
            serde_json::from_str(json).map_err(|error| format!("decode prompt images: {error}"))?;
        stored
            .into_iter()
            .map(|image| self.decode_prompt_image(image))
            .collect()
    }

    pub(super) fn decode_prompt_image(&self, image: StoredImage) -> Result<PromptImage, String> {
        match image {
            StoredImage::File {
                attachment,
                mime_type,
            } => {
                if !valid_attachment_hash(&attachment) {
                    return Err("Invalid image attachment hash".into());
                }
                Ok(PromptImage::from_file(
                    self.image_directory.join(attachment),
                    mime_type,
                ))
            }
            StoredImage::Inline(image) => Ok(image),
        }
    }
}

fn valid_attachment_hash(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_image(
    image: &PromptImage,
    output: &mut impl std::io::Write,
) -> Result<(String, usize), String> {
    let mut input: Box<dyn std::io::Read + '_> = match &image.path {
        Some(path) => Box::new(
            std::fs::File::open(path)
                .map_err(|error| format!("read image {}: {error}", path.display()))?,
        ),
        None => Box::new(base64::read::DecoderReader::new(
            image.data.as_bytes(),
            &base64::engine::general_purpose::STANDARD,
        )),
    };
    let mut hash = Sha256::new();
    let mut length = 0;
    let mut buffer = [0; 8 * 1024];
    loop {
        let count = match input.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(match &image.path {
                    Some(path) => format!("read image {}: {error}", path.display()),
                    None => format!("decode image: {error}"),
                });
            }
        };
        if count == 0 {
            break;
        }
        let bytes = &buffer[..count];
        hash.update(bytes);
        output
            .write_all(bytes)
            .map_err(|error| format!("write image: {error}"))?;
        length += count;
    }
    Ok((format!("{:x}", hash.finalize()), length))
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
