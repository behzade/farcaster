use gpui::AppContext as _;
use std::collections::HashMap;

use super::{ComposerImage, ComposerPaste, FarcasterApp, sessions::ComposerSessions};
use crate::app::infrastructure::persistence::ComposerAttachment;

impl FarcasterApp {
    pub(in crate::app) fn save_composer_attachments(&mut self, target: &str) {
        if self.composer.sessions.current_target() == target && self.has_composer_attachments() {
            self.engage_current_draft();
        }
        let pending = self
            .composer
            .pending_submissions
            .values()
            .filter(|pending| pending.submitted_target == target);
        let images = pending
            .clone()
            .flat_map(|pending| pending.images.iter())
            .chain(self.composer.images.get(target).into_iter().flatten())
            .map(|image| ComposerAttachment::Image(image.prompt.clone()));
        let files = pending
            .flat_map(|pending| pending.pastes.iter())
            .chain(self.composer.pastes.get(target).into_iter().flatten())
            .map(|paste| ComposerAttachment::TextFile {
                path: paste.path.clone(),
            });
        self.composer
            .sessions
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

impl FarcasterApp {
    pub(in crate::app) fn choose_composer_attachments(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let target = self.composer.sessions.current_target().to_owned();
        let selected = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images or text files".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selected.await else {
                return;
            };
            let attachments = cx
                .background_spawn(async move {
                    paths.into_iter().map(load_attachment).collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let mut errors = Vec::new();
                for attachment in attachments {
                    match attachment {
                        Ok(LoadedAttachment::Image(image)) => this
                            .composer
                            .images
                            .entry(target.clone())
                            .or_default()
                            .push(image),
                        Ok(LoadedAttachment::Text(text)) => this
                            .composer
                            .pastes
                            .entry(target.clone())
                            .or_default()
                            .push(text),
                        Err(error) => errors.push(error),
                    }
                }
                this.save_composer_attachments(&target);
                this.notify_composer(cx);
                if this.composer.sessions.current_target() == target {
                    this.composer.focus.focus(window, cx);
                }
                if !errors.is_empty() {
                    this.notify_workspace_error("Attachments", errors.join("\n"), cx);
                }
            });
        })
        .detach();
    }
}

enum LoadedAttachment {
    Image(ComposerImage),
    Text(ComposerPaste),
}

fn load_attachment(path: std::path::PathBuf) -> Result<LoadedAttachment, String> {
    use base64::Engine as _;
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if let Ok(format) = image::guess_format(&bytes) {
        return ComposerImage::from_prompt(crate::protocol::PromptImage::new(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            format.to_mime_type().into(),
        ))
        .map(LoadedAttachment::Image);
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| format!("{}: choose an image or UTF-8 text file", path.display()))?;
    super::pastes::store_attachment(&path, content).map(LoadedAttachment::Text)
}
