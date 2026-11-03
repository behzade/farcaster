use std::{borrow::Cow, fmt::Write as _, ops::Range};

use markdown::mdast::Node;

// User messages keep Markdown formatting, but source line breaks and raw HTML
// (including code such as Vec<T>) must remain visible. Only change prose nodes;
// code fences, inline code, links, and existing hard breaks keep their syntax.
pub(super) fn markdown(source: &str) -> Cow<'_, str> {
    let Ok(root) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return Cow::Borrowed(source);
    };
    let mut edits = Vec::new();
    collect_edits(&root, source, &mut edits);
    if edits.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut output = String::with_capacity(source.len());
    let mut offset = 0;
    for (range, replacement) in edits {
        output.push_str(&source[offset..range.start]);
        output.push_str(&replacement);
        offset = range.end;
    }
    output.push_str(&source[offset..]);
    Cow::Owned(output)
}

fn collect_edits(node: &Node, source: &str, edits: &mut Vec<(Range<usize>, String)>) {
    if let Some(position) = node.position() {
        let range = position.start.offset..position.end.offset;
        let raw = &source[range.clone()];
        if let Node::Html(html) = node {
            let mut literal = String::new();
            let raw = normalize_newlines(raw);
            let value = normalize_newlines(&html.value);
            let mut lines = value.split('\n');
            for (index, line) in raw.split('\n').enumerate() {
                if index > 0 {
                    literal.push_str("  \n");
                }
                // The source span includes quote/list prefixes on later lines;
                // the AST value excludes them. Keep those prefixes as Markdown.
                let (prefix, content) = lines
                    .next()
                    .and_then(|content| line.strip_suffix(content).map(|prefix| (prefix, content)))
                    .unwrap_or(("", line));
                literal.push_str(prefix);
                for character in content.chars() {
                    if character.is_ascii_punctuation() {
                        write!(literal, "&#{};", u32::from(character))
                            .expect("writing to a String cannot fail");
                    } else {
                        literal.push(character);
                    }
                }
            }
            edits.push((range, literal));
        } else if matches!(node, Node::Text(_)) && raw.contains(['\r', '\n']) {
            edits.push((range, hard_breaks(raw)));
        }
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_edits(child, source, edits);
        }
    }
}

fn hard_breaks(text: &str) -> String {
    normalize_newlines(text).replace('\n', "  \n")
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
#[path = "user_message_tests.rs"]
mod tests;
