use super::*;

pub(super) const MARKDOWN_CHUNK_TARGET_BYTES: usize = 2 * 1024;
pub(in crate::app::views::transcript) const MARKDOWN_CHUNK_HARD_BYTES: usize = 8 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::views::transcript) struct MarkdownFence {
    opening_start: usize,
    opening_end: usize,
    marker: char,
    marker_len: usize,
    indent_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FenceContinuation {
    fence: MarkdownFence,
    prepend: bool,
    append: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::views::transcript) struct MarkdownChunk {
    pub(in crate::app::views::transcript) start: usize,
    pub(in crate::app::views::transcript) end: usize,
    pub(in crate::app::views::transcript) fence: Option<FenceContinuation>,
}

pub(super) fn markdown_needs_chunks(text: &str) -> bool {
    text.len() > MARKDOWN_CHUNK_HARD_BYTES || text.lines().take(65).count() > 64
}

pub(in crate::app::views::transcript) fn markdown_chunks(text: &str) -> Vec<MarkdownChunk> {
    let mut chunks = Vec::new();
    let mut outside_start = 0;
    let mut offset = 0;
    let mut fence = None;
    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if let Some(opening) = fence {
            if markdown_fence_closes(line, opening) {
                append_fenced_markdown_chunks(text, opening, line_start, offset, true, &mut chunks);
                fence = None;
                outside_start = offset;
            }
        } else if let Some(opening) = markdown_fence(line, line_start, offset) {
            append_plain_markdown_chunks(text, outside_start, line_start, &mut chunks);
            fence = Some(opening);
        }
    }

    if let Some(opening) = fence {
        append_fenced_markdown_chunks(text, opening, text.len(), text.len(), false, &mut chunks);
    } else {
        append_plain_markdown_chunks(text, outside_start, text.len(), &mut chunks);
    }
    if chunks.is_empty() {
        chunks.push(plain_markdown_chunk(0, text.len()));
    }
    // Keep inline links/images atomic even when their URLs exceed the normal chunk size.
    if text.contains('[')
        && let Ok(root) = markdown::to_mdast(text, &markdown::ParseOptions::gfm())
    {
        let mut spans = Vec::new();
        protected_spans(&root, &mut spans);
        let mut merged: Vec<MarkdownChunk> = Vec::new();
        for chunk in chunks {
            if let Some(previous) = merged.last_mut()
                && previous.fence.is_none()
                && chunk.fence.is_none()
                && spans
                    .iter()
                    .any(|range| range.start < chunk.start && chunk.start < range.end)
            {
                previous.end = chunk.end;
            } else {
                merged.push(chunk);
            }
        }
        return merged;
    }
    chunks
}

fn protected_spans(node: &markdown::mdast::Node, spans: &mut Vec<std::ops::Range<usize>>) {
    use markdown::mdast::Node;
    if matches!(
        node,
        Node::Image(_) | Node::Link(_) | Node::ImageReference(_) | Node::LinkReference(_)
    ) && let Some(position) = node.position()
    {
        spans.push(position.start.offset..position.end.offset);
    }
    if let Some(children) = node.children() {
        for child in children {
            protected_spans(child, spans);
        }
    }
}

fn plain_markdown_chunk(start: usize, end: usize) -> MarkdownChunk {
    MarkdownChunk {
        start,
        end,
        fence: None,
    }
}

fn append_plain_markdown_chunks(
    text: &str,
    mut start: usize,
    end: usize,
    chunks: &mut Vec<MarkdownChunk>,
) {
    if start >= end {
        return;
    }
    let mut line_end = start;
    for line in text[start..end].split_inclusive('\n') {
        line_end += line.len();
        while line_end - start >= MARKDOWN_CHUNK_HARD_BYTES {
            let split = hard_markdown_break(text, start, start + MARKDOWN_CHUNK_HARD_BYTES);
            chunks.push(plain_markdown_chunk(start, split));
            start = split;
        }
        if line_end - start >= MARKDOWN_CHUNK_TARGET_BYTES && line.trim().is_empty() {
            chunks.push(plain_markdown_chunk(start, line_end));
            start = line_end;
        }
    }
    if start < end {
        chunks.push(plain_markdown_chunk(start, end));
    }
}

fn append_fenced_markdown_chunks(
    text: &str,
    fence: MarkdownFence,
    closing_start: usize,
    fence_end: usize,
    closed: bool,
    chunks: &mut Vec<MarkdownChunk>,
) {
    const FENCED_CHUNK_LINES: usize = 64;
    // A presentation action must receive the whole document, including its scripts.
    let opening = &text[fence.opening_start..fence.opening_end];
    let language = opening.trim().trim_start_matches(fence.marker).trim();
    if crate::app::attachment::resolve(crate::app::attachment::AttachmentSource::Code(language))
        .source
        .is_some()
    {
        chunks.push(plain_markdown_chunk(fence.opening_start, fence_end));
        return;
    }
    if fence_end - fence.opening_start <= MARKDOWN_CHUNK_HARD_BYTES
        && text[fence.opening_end..closing_start].lines().count() <= FENCED_CHUNK_LINES
    {
        chunks.push(plain_markdown_chunk(fence.opening_start, fence_end));
        return;
    }

    let mut body = Vec::new();
    let mut start = fence.opening_end;
    let mut end = start;
    let mut lines = 0;
    for line in text[start..closing_start].split_inclusive('\n') {
        end += line.len();
        lines += 1;
        while end - start >= MARKDOWN_CHUNK_HARD_BYTES {
            let split = hard_markdown_break(text, start, start + MARKDOWN_CHUNK_HARD_BYTES);
            body.push(plain_markdown_chunk(start, split));
            start = split;
            lines = 0;
        }
        if lines >= FENCED_CHUNK_LINES || end - start >= MARKDOWN_CHUNK_TARGET_BYTES {
            body.push(plain_markdown_chunk(start, end));
            start = end;
            lines = 0;
        }
    }
    if start < closing_start {
        body.push(plain_markdown_chunk(start, closing_start));
    }
    if body.is_empty() {
        chunks.push(plain_markdown_chunk(fence.opening_start, fence_end));
        return;
    }

    let last = body.len() - 1;
    for (index, body_chunk) in body.into_iter().enumerate() {
        let first = index == 0;
        let final_chunk = index == last;
        chunks.push(MarkdownChunk {
            start: if first {
                fence.opening_start
            } else {
                body_chunk.start
            },
            end: if final_chunk && closed {
                fence_end
            } else {
                body_chunk.end
            },
            fence: Some(FenceContinuation {
                fence,
                prepend: !first,
                append: !final_chunk || !closed,
            }),
        });
    }
}

pub(in crate::app::views::transcript) fn markdown_chunk_text(
    text: &str,
    chunk: MarkdownChunk,
) -> Cow<'_, str> {
    let Some(continuation) = chunk.fence else {
        let body = &text[chunk.start..chunk.end];
        if text.contains("]:")
            && let Ok(root) = markdown::to_mdast(text, &markdown::ParseOptions::gfm())
        {
            let mut definitions = String::new();
            collect_definitions(&root, text, &mut definitions);
            if !definitions.is_empty() {
                return Cow::Owned(format!("{body}\n\n{definitions}"));
            }
        }
        return Cow::Borrowed(body);
    };
    if !continuation.prepend && !continuation.append {
        return Cow::Borrowed(&text[chunk.start..chunk.end]);
    }
    let fence = continuation.fence;
    let mut rendered = String::with_capacity(
        chunk.end - chunk.start + fence.opening_end - fence.opening_start + fence.marker_len + 2,
    );
    if continuation.prepend {
        rendered.push_str(&text[fence.opening_start..fence.opening_end]);
    }
    rendered.push_str(&text[chunk.start..chunk.end]);
    if continuation.append {
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(&text[fence.opening_start..fence.opening_start + fence.indent_len]);
        rendered.extend(std::iter::repeat_n(fence.marker, fence.marker_len));
        rendered.push('\n');
    }
    Cow::Owned(rendered)
}

fn hard_markdown_break(text: &str, start: usize, mut limit: usize) -> usize {
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let minimum = start + MARKDOWN_CHUNK_TARGET_BYTES;
    text[start..limit]
        .char_indices()
        .rev()
        .find(|(offset, char)| start + offset >= minimum && char.is_whitespace())
        .map_or(limit, |(offset, char)| start + offset + char.len_utf8())
}

pub(in crate::app::views::transcript) fn markdown_fence(
    line: &str,
    opening_start: usize,
    opening_end: usize,
) -> Option<MarkdownFence> {
    let indent_len = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent_len > 3 {
        return None;
    }
    let trimmed = &line[indent_len..];
    let marker = trimmed.chars().next()?;
    let marker_len = trimmed.chars().take_while(|char| *char == marker).count();
    ((marker == '`' || marker == '~') && marker_len >= 3).then_some(MarkdownFence {
        opening_start,
        opening_end,
        marker,
        marker_len,
        indent_len,
    })
}

pub(in crate::app::views::transcript) fn markdown_fence_closes(
    line: &str,
    fence: MarkdownFence,
) -> bool {
    let indent_len = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent_len > 3 {
        return false;
    }
    let trimmed = line[indent_len..].trim_end();
    let run = trimmed
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    run >= fence.marker_len && trimmed.chars().skip(run).all(char::is_whitespace)
}

fn collect_definitions(node: &markdown::mdast::Node, text: &str, out: &mut String) {
    if matches!(node, markdown::mdast::Node::Definition(_))
        && let Some(position) = node.position()
    {
        out.push_str(&text[position.start.offset..position.end.offset]);
        out.push('\n');
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, text, out);
        }
    }
}
