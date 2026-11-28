use std::ops::Range;

use markdown::mdast::Node;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MarkdownStructure {
    pub block_starts: Vec<usize>,
    pub links: Vec<Range<usize>>,
    pub definitions: Vec<Range<usize>>,
}

impl MarkdownStructure {
    pub fn parse(text: &str) -> Option<Self> {
        let root = markdown::to_mdast(text, &markdown::ParseOptions::gfm()).ok()?;
        let mut structure = Self::default();
        for block in root.children()?.iter().skip(1) {
            let offset = block.position()?.start.offset;
            if let Some(end) = text[..offset].rfind(['\n', '\r']) {
                structure.block_starts.push(end + 1);
            }
        }
        structure.collect(&root);
        Some(structure)
    }

    fn collect(&mut self, node: &Node) {
        if let Some(pos) = node.position() {
            let range = pos.start.offset..pos.end.offset;
            match node {
                Node::Image(_)
                | Node::Link(_)
                | Node::ImageReference(_)
                | Node::LinkReference(_) => self.links.push(range),
                Node::Definition(_) => self.definitions.push(range),
                _ => {}
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                self.collect(child);
            }
        }
    }

    pub fn valid_for(&self, text: &str) -> bool {
        self.block_starts
            .iter()
            .all(|&n| n > 0 && text.is_char_boundary(n))
            && self.block_starts.windows(2).all(|ns| ns[0] < ns[1])
            && self
                .links
                .iter()
                .chain(&self.definitions)
                .all(|range| text.get(range.clone()).is_some())
    }
}

pub type MarkdownIndex = std::collections::BTreeMap<usize, MarkdownStructure>;
