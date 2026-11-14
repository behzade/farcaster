use std::{borrow::Cow, cell::RefCell, collections::HashMap, hash::Hash, rc::Rc};

use gpui::{AppContext as _, Entity};
use gpui_component::text::TextViewState;

mod user_message;

const MAX_CACHED_MARKDOWN_ROWS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MarkdownRowKind {
    Item,
    MessageChunk,
    StreamChunk,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MarkdownStateKey {
    item: usize,
    block: usize,
    revision: usize,
    kind: MarkdownRowKind,
}

impl MarkdownStateKey {
    pub(crate) fn item(item: usize, revision: usize) -> Self {
        Self {
            item,
            block: 0,
            revision,
            kind: MarkdownRowKind::Item,
        }
    }

    pub(crate) fn message_chunk(item: usize, block: usize, revision: usize) -> Self {
        Self {
            item,
            block,
            revision,
            kind: MarkdownRowKind::MessageChunk,
        }
    }

    pub(crate) fn stream_chunk(item: usize, block: usize, revision: usize) -> Self {
        Self {
            item,
            block,
            revision,
            kind: MarkdownRowKind::StreamChunk,
        }
    }
}

struct RecentCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    clock: u64,
    capacity: usize,
}

impl<K: Copy + Eq + Hash, V: Clone> RecentCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity,
        }
    }

    fn get_or_insert_with(&mut self, key: K, create: impl FnOnce() -> V) -> (V, bool) {
        self.clock = self.clock.wrapping_add(1);
        if let Some((value, last_used)) = self.entries.get_mut(&key) {
            *last_used = self.clock;
            return (value.clone(), true);
        }

        let value = create();
        if self.entries.len() >= self.capacity
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| last_used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, (value.clone(), self.clock));
        (value, false)
    }
}

#[derive(Clone)]
pub(crate) struct TranscriptMarkdownCache {
    states: Rc<RefCell<RecentCache<(MarkdownStateKey, bool), Entity<TextViewState>>>>,
}

impl Default for TranscriptMarkdownCache {
    fn default() -> Self {
        Self {
            states: Rc::new(RefCell::new(RecentCache::new(MAX_CACHED_MARKDOWN_ROWS))),
        }
    }
}

impl TranscriptMarkdownCache {
    pub(crate) fn state(
        &self,
        key: MarkdownStateKey,
        text: &str,
        user_message: bool,
        cx: &mut gpui::App,
    ) -> Entity<TextViewState> {
        self.state_with(key, || Cow::Borrowed(text), user_message, cx)
    }

    pub(crate) fn state_with<'a>(
        &self,
        key: MarkdownStateKey,
        text: impl FnOnce() -> Cow<'a, str>,
        user_message: bool,
        cx: &mut gpui::App,
    ) -> Entity<TextViewState> {
        let (state, hit) = self
            .states
            .borrow_mut()
            .get_or_insert_with((key, user_message), || {
                let text = text();
                let _timing = crate::app::infrastructure::performance::OperationTiming::new(
                    crate::app::infrastructure::performance::OperationKind::MarkdownParse,
                    text.len(),
                );
                let rendered = if user_message {
                    user_message::markdown(&text)
                } else {
                    Cow::Borrowed(text.as_ref())
                };
                let rendered = clickable_images(&rendered);
                cx.new(|cx| TextViewState::markdown(&rendered, cx))
            });
        if hit {
            crate::app::infrastructure::performance::count_markdown_cache_hit();
        }
        state
    }
}

pub(super) fn clickable_images(source: &str) -> Cow<'_, str> {
    use markdown::mdast::Node;
    fn definitions(node: &Node, urls: &mut HashMap<String, String>) {
        if let Node::Definition(definition) = node {
            urls.entry(definition.identifier.clone())
                .or_insert_with(|| definition.url.clone());
        }
        if let Some(children) = node.children() {
            for child in children {
                definitions(child, urls);
            }
        }
    }
    fn visit(
        node: &Node,
        in_link: bool,
        allow_image_blocks: bool,
        source: &str,
        urls: &HashMap<String, String>,
        edits: &mut Vec<(std::ops::Range<usize>, String)>,
    ) {
        // A soft newline is still part of the same Markdown paragraph. An
        // image on its own source line belongs below its caption, not beside it.
        let image_line = allow_image_blocks
            && (matches!(node, Node::Image(_) | Node::ImageReference(_)) && !in_link
                || matches!(node, Node::Link(link) if link.children.len() == 1 && matches!(link.children[0], Node::Image(_) | Node::ImageReference(_)))
                || matches!(node, Node::LinkReference(link) if link.children.len() == 1 && matches!(link.children[0], Node::Image(_) | Node::ImageReference(_))));
        if image_line && let Some(position) = node.position() {
            let start = position.start.offset;
            let end = position.end.offset;
            let before = source[..start].rsplit('\n').next().unwrap_or_default();
            let after = source[end..].split('\n').next().unwrap_or_default();
            if before.trim().is_empty() && after.trim().is_empty() {
                // Base's image-only paragraph path ignores explicit height
                // while loading. Keep this in its measured inline flow so a
                // retained row cannot cache a zero-height preview.
                edits.push((start..start, "\n\n\u{200b}".into()));
                edits.push((end..end, "\n\n".into()));
            }
        }
        let url = match node {
            Node::Image(image) => Some(image.url.as_str()),
            Node::ImageReference(image) => urls.get(&image.identifier).map(String::as_str),
            _ => None,
        };
        if let Some(url) = url
            && let Some(position) = node.position()
        {
            let range = position.start.offset..position.end.offset;
            fn attr(value: &str) -> String {
                value
                    .replace('&', "&amp;")
                    .replace('"', "&quot;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            }
            let (alt, title) = match node {
                Node::Image(image) => (image.alt.as_str(), image.title.as_deref()),
                Node::ImageReference(image) => (image.alt.as_str(), None),
                _ => unreachable!(),
            };
            let title = title
                .map(|title| format!(" title=\"{}\"", attr(title)))
                .unwrap_or_default();
            // Reserve the actual aspect ratio before image loading so retained
            // transcript rows have stable bounds from their first frame.
            let bounds = crate::app::workspace::files::image_preview_dimensions(url)
                .map(|(width, height)| {
                    let scale = (320.0 / f64::from(width)).min(480.0 / f64::from(height));
                    format!(
                        "width=\"{}\" height=\"{}\"",
                        (f64::from(width) * scale).round().max(1.0) as u32,
                        (f64::from(height) * scale).round().max(1.0) as u32
                    )
                })
                .unwrap_or_else(|| "width=\"320\"".into());
            let image = format!(
                "<img src=\"{}\" alt=\"{}\" {bounds}{title}>",
                attr(url),
                attr(alt)
            );
            edits.push((
                range.clone(),
                if in_link {
                    image
                } else {
                    format!("[{image}](<{}>)", url.replace('>', "%3E"))
                },
            ));
            return;
        }
        let link = match node {
            Node::Link(link) => Some(link.url.as_str()),
            Node::LinkReference(link) => urls.get(&link.identifier).map(String::as_str),
            _ => None,
        };
        if let Some(url) = link
            && let Some((path, None)) =
                crate::app::workspace::files::local_file_target(url, std::path::Path::new("/"))
            && crate::app::attachment::resolve(crate::app::attachment::AttachmentSource::File(
                &path,
            ))
            .source
            .is_some()
            && let Some(position) = node.position()
        {
            let end = position.end.offset;
            let base = url.split('#').next().unwrap_or(url);
            edits.push((
                end..end,
                format!(" · [Edit source](<{}#L1>)", base.replace('>', "%3E")),
            ));
        }
        if let Some(children) = node.children() {
            for child in children {
                visit(
                    child,
                    in_link || link.is_some(),
                    allow_image_blocks
                        && !matches!(node, Node::List(_) | Node::Blockquote(_) | Node::Table(_)),
                    source,
                    urls,
                    edits,
                );
            }
        }
    }
    if !source.contains('[') {
        return Cow::Borrowed(source);
    }
    let Ok(root) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return Cow::Borrowed(source);
    };
    let mut urls = HashMap::new();
    definitions(&root, &mut urls);
    let mut edits = Vec::new();
    visit(&root, false, true, source, &urls, &mut edits);
    if edits.is_empty() {
        return Cow::Borrowed(source);
    }
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let mut out = source.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(range, &replacement);
    }
    Cow::Owned(out)
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
