use std::path::{Path, PathBuf};

use gpui::{ClickEvent, MouseButton, WeakEntity};
use gpui_component::text::TextView;
use url::Url;

use crate::app::FarcasterApp;

pub(super) fn with_file_links(text: TextView, entity: WeakEntity<FarcasterApp>) -> TextView {
    text.on_link_click(move |url, event, window, cx| {
        let activate = match event {
            ClickEvent::Mouse(click) => {
                matches!(click.up.button, MouseButton::Left | MouseButton::Middle)
            }
            ClickEvent::Keyboard(_) => true,
            ClickEvent::Touch(click) => !click.long_press,
        };
        if !activate || url.is_empty() || url.starts_with('#') {
            return;
        }
        let _ = entity.update(cx, |this, cx| {
            if let Some((path, line)) = local_file_target(url, &this.workspace_project()) {
                this.open_file_editor_at_line(path, line, window, cx);
            } else {
                cx.open_url(url);
            }
        });
    })
}

fn local_file_target(link: &str, project: &Path) -> Option<(PathBuf, Option<u64>)> {
    if link.is_empty() || link.starts_with('#') || link.starts_with("//") {
        return None;
    }
    let (path, fragment) = link
        .split_once('#')
        .map_or((link, None), |(path, fragment)| (path, Some(fragment)));
    let fragment_line = fragment.and_then(|fragment| {
        let start = fragment.strip_prefix('L')?.split('-').next()?;
        positive_line(start)
    });
    let (path, line) = split_line_suffix(path);
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

#[cfg(test)]
#[path = "links_tests.rs"]
mod tests;
