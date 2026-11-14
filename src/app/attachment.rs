use std::path::Path;

use gpui::ImageFormat;

pub(super) const IMAGE_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const IMAGE_FALLBACK: OpenAction = OpenAction::System;

pub(super) enum AttachmentSource<'a> {
    File(&'a Path),
    Image(ImageFormat),
    Code(&'a str),
    Url(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AttachmentView {
    File,
    Image,
    Code,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenAction {
    PreviewImage(ImageFormat),
    Browser,
    Editor,
    System,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AttachmentRule {
    pub view: AttachmentView,
    pub open: OpenAction,
    pub source: Option<OpenAction>,
    pub fallback: Option<OpenAction>,
}

impl AttachmentRule {
    pub fn open_at(self, line: Option<u64>) -> OpenAction {
        if line.is_some() {
            self.source.unwrap_or(self.open)
        } else {
            self.open
        }
    }

    pub fn preview_action(self, bytes: u64, decoded: bool) -> OpenAction {
        if bytes > IMAGE_PREVIEW_BYTES || !decoded {
            self.fallback.unwrap_or(self.open)
        } else {
            self.open
        }
    }
}

// File references stay compact cards, even when clicking one previews an image.
// Encoded images share the same card in the composer and transcript. Code blocks
// keep their code view and expose the same HTML open/source actions as files.
pub(super) fn resolve(source: AttachmentSource<'_>) -> AttachmentRule {
    use AttachmentView::{Code, File, Image};
    use OpenAction::{Browser, Editor, PreviewImage, System};

    let (view, open) = match source {
        AttachmentSource::Url(link) => {
            let open = match url::Url::parse(link).as_ref().map(url::Url::scheme) {
                Ok("http" | "https") => Browser,
                Ok("mailto") => System,
                _ => OpenAction::Unavailable,
            };
            return AttachmentRule {
                view: File,
                open,
                source: None,
                fallback: None,
            };
        }
        AttachmentSource::Image(format) => (Image, PreviewImage(format)),
        AttachmentSource::Code(language) => {
            let action = extension_action(&language.to_ascii_lowercase());
            (Code, if action == Browser { Browser } else { Editor })
        }
        AttachmentSource::File(path) => {
            let extension = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let action = if extension.is_empty()
                && path.file_name().is_some_and(|name| {
                    matches!(
                        name.to_str(),
                        Some("Dockerfile" | "Makefile" | "Justfile" | "LICENSE")
                    ) || name.to_string_lossy().starts_with('.')
                }) {
                Editor
            } else {
                extension_action(&extension)
            };
            (File, action)
        }
    };
    AttachmentRule {
        view,
        open,
        source: (open == Browser).then_some(Editor),
        fallback: matches!(open, PreviewImage(_)).then_some(IMAGE_FALLBACK),
    }
}

fn extension_action(extension: &str) -> OpenAction {
    use ImageFormat::*;
    use OpenAction::*;
    match extension {
        "png" => PreviewImage(Png),
        "jpg" | "jpeg" => PreviewImage(Jpeg),
        "gif" => PreviewImage(Gif),
        "webp" => PreviewImage(Webp),
        "svg" => PreviewImage(Svg),
        "bmp" => PreviewImage(Bmp),
        "tif" | "tiff" => PreviewImage(Tiff),
        "ico" => PreviewImage(Ico),
        "html" | "htm" => Browser,
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "json" | "jsonc" | "toml" | "yaml" | "yml"
        | "xml" | "css" | "scss" | "sh" | "bash" | "zsh" | "nix" | "go" | "c" | "h" | "cpp"
        | "hpp" | "java" | "kt" | "swift" | "rb" | "lua" | "sql" | "md" | "mdx" | "txt" | "log"
        | "csv" | "tsv" | "ini" | "conf" | "cfg" | "env" | "diff" | "patch" => Editor,
        _ => System,
    }
}

#[cfg(test)]
#[path = "attachment_tests.rs"]
mod tests;
