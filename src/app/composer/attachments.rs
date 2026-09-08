use std::collections::HashMap;

use super::{ComposerImage, ComposerPaste, FarcasterApp, sessions::ComposerSessions};
use crate::app::infrastructure::persistence::ComposerAttachment;

impl FarcasterApp {
    pub(in crate::app) fn save_composer_attachments(&mut self, target: &str) {
        // Keep in-flight attachments recoverable until the runtime accepts the submission.
        let pending = self.pending_submissions.get(target);
        let images = pending
            .into_iter()
            .flat_map(|pending| pending.images.iter())
            .chain(self.composer_images.get(target).into_iter().flatten())
            .map(|image| ComposerAttachment::Image(image.prompt.clone()));
        let files = pending
            .into_iter()
            .flat_map(|pending| pending.pastes.iter())
            .chain(self.composer_pastes.get(target).into_iter().flatten())
            .map(|paste| ComposerAttachment::TextFile {
                path: paste.path.clone(),
            });
        self.composer_sessions
            .set_attachments(target, images.chain(files).collect());
    }
}

pub(in crate::app) fn restore(
    sessions: &ComposerSessions,
) -> (
    HashMap<String, Vec<ComposerImage>>,
    HashMap<String, Vec<ComposerPaste>>,
) {
    let mut images = HashMap::<String, Vec<ComposerImage>>::new();
    let mut pastes = HashMap::<String, Vec<ComposerPaste>>::new();
    for (target, attachments) in sessions.saved_attachments() {
        for attachment in attachments {
            let result = match attachment {
                ComposerAttachment::Image(image) => ComposerImage::from_prompt(image.clone())
                    .map(|image| images.entry(target.clone()).or_default().push(image)),
                ComposerAttachment::TextFile { path } => ComposerPaste::from_path(path.clone())
                    .map(|paste| pastes.entry(target.clone()).or_default().push(paste)),
            };
            if let Err(error) = result {
                zlog::error!("Cannot restore composer attachment for {target}: {error}");
            }
        }
    }
    (images, pastes)
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
