use std::{
    collections::VecDeque,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use base64::Engine as _;
use gpui::{Context, Image, ImageFormat, ImageSource, Window};
use url::Url;

use super::FarcasterApp;
use crate::app::attachment::{self, AttachmentSource, IMAGE_PREVIEW_BYTES, OpenAction};

pub(in crate::app) fn local_file_target(
    link: &str,
    project: &Path,
) -> Option<(PathBuf, Option<u64>)> {
    if link.is_empty() || link.starts_with('#') || link.starts_with("//") {
        return None;
    }
    let (path, fragment) = link
        .split_once('#')
        .map_or((link, None), |(path, fragment)| (path, Some(fragment)));
    let fragment_line =
        fragment.and_then(|s| positive_line(s.strip_prefix('L')?.split('-').next()?));
    let (path, line) = split_line_suffix(path);
    // A sandbox URI is a local artifact path, not a registered OS URL scheme.
    let path = path.strip_prefix("sandbox:").unwrap_or(path);
    let url = match Url::parse(path) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            Url::from_directory_path(project).ok()?.join(path).ok()?
        }
        Err(_) => return None,
    };
    if url.scheme() != "file" {
        return None;
    }
    Some((url.to_file_path().ok()?, fragment_line.or(line)))
}

fn positive_line(value: &str) -> Option<u64> {
    value.parse().ok().filter(|line| *line > 0)
}
fn split_line_suffix(path: &str) -> (&str, Option<u64>) {
    let Some((prefix, last)) = path.rsplit_once(':') else {
        return (path, None);
    };
    let Some(last) = positive_line(last) else {
        return (path, None);
    };
    if let Some((path, line)) = prefix.rsplit_once(':')
        && let Some(line) = positive_line(line)
    {
        return (path, Some(line));
    }
    (prefix, Some(last))
}

pub(in crate::app) fn data_image(uri: &str) -> Option<Arc<Image>> {
    let (mime, data) = uri.strip_prefix("data:")?.split_once(";base64,")?;
    type Cache = VecDeque<(String, Arc<Image>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().ok()?;
    if let Some(index) = cache.iter().position(|(key, _)| key == uri) {
        let entry = cache.remove(index)?;
        let image = entry.1.clone();
        cache.push_back(entry);
        return Some(image);
    }
    if data.len() as u64 > IMAGE_PREVIEW_BYTES * 4 / 3 + 4 {
        return None;
    }
    let image = Arc::new(Image::from_bytes(
        ImageFormat::from_mime_type(mime)?,
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?,
    ));
    let size = uri.len() + image.bytes().len();
    while !cache.is_empty()
        && cache
            .iter()
            .map(|(key, image)| key.len() + image.bytes().len())
            .sum::<usize>()
            + size
            > IMAGE_PREVIEW_BYTES as usize
    {
        cache.pop_front();
    }
    if size <= IMAGE_PREVIEW_BYTES as usize {
        cache.push_back((uri.to_owned(), image.clone()));
    }
    Some(image)
}

// Read only a bounded header; this runs once when a Markdown row is parsed.
// Stable bounds avoid retaining an empty image row while its bytes load.
pub(in crate::app) fn image_preview_dimensions(uri: &str) -> Option<(u32, u32)> {
    let bytes = if let Some(image) = data_image(uri) {
        image.bytes()[..image.bytes().len().min(64 * 1024)].to_vec()
    } else {
        let (path, _) = local_file_target(uri, Path::new("/"))?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .ok()?;
        bytes
    };
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

pub(in crate::app) fn image_source(uri: &str, project: &Path) -> ImageSource {
    if let Some(image) = data_image(uri) {
        return image.into();
    }
    if let Some((path, _)) = local_file_target(uri, project) {
        return path.into();
    }
    gpui::SharedUri::from(uri.to_owned()).into()
}

fn artifact_path(root: &Path, prefix: &str, extension: &str) -> Result<PathBuf, String> {
    let directory = root.join("artifacts");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    Ok(directory.join(format!("{prefix}-{}.{extension}", uuid::Uuid::new_v4())))
}

impl FarcasterApp {
    pub(in crate::app) fn open_transcript_link(
        &mut self,
        link: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(image) = data_image(link) {
            self.open_image_preview(image, 0, 1, window, cx);
        } else if link.starts_with("data:image/") && link.contains(";base64,") {
            self.open_data_image_externally(link.to_owned(), window, cx);
        } else if let Some((path, line)) = local_file_target(link, &self.workspace_project()) {
            if attachment::resolve(AttachmentSource::File(&path)).open_at(line)
                == OpenAction::Browser
            {
                let (before_fragment, fragment) = link
                    .split_once('#')
                    .map_or((link, None), |(path, fragment)| {
                        (path, Some(fragment.to_owned()))
                    });
                let query = before_fragment
                    .split_once('?')
                    .map(|(_, query)| query.to_owned());
                self.open_browser_target(path, query, fragment, window, cx);
            } else {
                self.open_transcript_file(path, line, window, cx);
            }
        } else if attachment::resolve(AttachmentSource::Url(link)).open != OpenAction::Unavailable {
            cx.open_url(link);
        } else {
            self.notify_workspace_error(
                "Open attachment",
                "This attachment has no usable local path or web URL.".into(),
                cx,
            );
        }
    }

    pub(in crate::app) fn open_transcript_file(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = if path.is_absolute() {
            path
        } else {
            self.workspace_project().join(path)
        };
        let project = self.workspace_project();
        let target = self.composer.sessions.current_target().to_owned();
        let rule = attachment::resolve(AttachmentSource::File(&path));
        let read = cx.background_executor().spawn(async move {
            let metadata = std::fs::metadata(&path)
                .map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
            let image = if let OpenAction::PreviewImage(format) = rule.open {
                if metadata.len() > IMAGE_PREVIEW_BYTES {
                    return Ok((path, None));
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .map_err(|e| e.to_string())?
                    .take(IMAGE_PREVIEW_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > IMAGE_PREVIEW_BYTES {
                    return Ok((path, None));
                }
                Some(Arc::new(Image::from_bytes(format, bytes)))
            } else {
                None
            };
            Ok((path, image))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result: Result<_, String> = read.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.workspace_project() != project
                    || this.composer.sessions.current_target() != target
                {
                    return;
                }
                match result {
                    Err(error) => this.notify_workspace_error("Open attachment", error, cx),
                    Ok((_path, Some(image))) => this.open_image_preview(image, 0, 1, window, cx),
                    Ok((path, None)) => this.open_attachment_action(
                        path,
                        line,
                        rule.fallback.unwrap_or(rule.open_at(line)),
                        window,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    pub(in crate::app) fn open_attachment_action(
        &mut self,
        path: PathBuf,
        line: Option<u64>,
        action: OpenAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            OpenAction::PreviewImage(_) => self.open_transcript_file(path, line, window, cx),
            OpenAction::Browser => self.open_in_browser(path, window, cx),
            OpenAction::Editor => self.open_file_editor_at_line(path, line, window, cx),
            OpenAction::System => cx.open_with_system(&path),
            OpenAction::Unavailable => self.notify_workspace_error(
                "Open attachment",
                "This attachment has no usable local path or web URL.".into(),
                cx,
            ),
        }
    }

    pub(in crate::app) fn open_in_browser(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_browser_target(path, None, None, window, cx);
    }

    fn open_browser_target(
        &mut self,
        path: PathBuf,
        query: Option<String>,
        fragment: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_executor().spawn(async move {
            let mut url = super::html_preview::url(&path)?;
            url.set_query(query.as_deref());
            url.set_fragment(fragment.as_deref());
            browser_open(&url)
        });
        cx.spawn_in(window, async move |weak, cx| {
            if let Err(error) = task.await {
                let _ = weak.update_in(cx, |this, _, cx| {
                    this.notify_workspace_error("Open in browser", error, cx)
                });
            }
        })
        .detach();
    }

    pub(in crate::app) fn open_html_code(
        &mut self,
        source: String,
        action: OpenAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = crate::app::infrastructure::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let path = artifact_path(&root?, "presentation", "html")?;
            std::fs::write(&path, source).map_err(|e| e.to_string())?;
            Ok::<_, String>(path)
        });
        self.open_artifact(task, action, "HTML preview", window, cx);
    }

    fn open_data_image_externally(
        &mut self,
        uri: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = crate::app::infrastructure::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let (mime, data) = uri
                .strip_prefix("data:")
                .and_then(|s| s.split_once(";base64,"))
                .ok_or("Invalid image data")?;
            let format = ImageFormat::from_mime_type(mime).ok_or("Unsupported image format")?;
            let path = artifact_path(&root?, "image", format.extension())?;
            let mut file =
                tempfile::NamedTempFile::new_in(path.parent().expect("artifact directory"))
                    .map_err(|e| e.to_string())?;
            let mut decoded = base64::read::DecoderReader::new(
                data.as_bytes(),
                &base64::engine::general_purpose::STANDARD,
            );
            std::io::copy(&mut decoded, &mut file).map_err(|e| e.to_string())?;
            file.persist(&path).map_err(|e| e.to_string())?;
            Ok::<_, String>(path)
        });
        self.open_artifact(task, attachment::IMAGE_FALLBACK, "Open image", window, cx);
    }

    pub(in crate::app) fn open_image_externally(
        &mut self,
        image: Arc<Image>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_image_action(image, OpenAction::System, window, cx);
    }

    pub(in crate::app) fn open_image_action(
        &mut self,
        image: Arc<Image>,
        action: OpenAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = crate::app::infrastructure::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let path = artifact_path(&root?, "image", image.format().extension())?;
            std::fs::write(&path, image.bytes()).map_err(|e| e.to_string())?;
            Ok::<_, String>(path)
        });
        self.open_artifact(task, action, "Open image externally", window, cx);
    }

    fn open_artifact(
        &mut self,
        task: gpui::Task<Result<PathBuf, String>>,
        action: OpenAction,
        title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| match result {
                Ok(path) => this.open_attachment_action(path, None, action, window, cx),
                Err(error) => this.notify_workspace_error(title, error, cx),
            });
        })
        .detach();
    }
}

#[cfg(target_os = "macos")]
fn browser_open(target: &Url) -> Result<(), String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let url = NSURL::URLWithString(&NSString::from_str("https://example.invalid"))
        .ok_or("Cannot find the default browser")?;
    let browser = NSWorkspace::sharedWorkspace()
        .URLForApplicationToOpenURL(&url)
        .and_then(|url| url.path())
        .ok_or("No default browser is configured")?;
    let status = std::process::Command::new("open")
        .arg("-a")
        .arg(browser.to_string())
        .arg("--")
        .arg(target.as_str())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Browser opener exited with {status}"))
    }
}

#[cfg(not(target_os = "macos"))]
fn browser_open(target: &Url) -> Result<(), String> {
    let status = std::process::Command::new("xdg-open")
        .arg(target.as_str())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Browser opener exited with {status}"))
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
