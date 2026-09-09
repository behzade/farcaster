use std::io::{self, Write};

const MAX_BYTES: usize = 4 * 1024;
const MAX_LINES: usize = 40;
const TRUNCATION_NOTICE: &str = "\n\n[Preview truncated. Copy the tool activity for full details.]";

/// Bounds work before Markdown parsing and text layout, including JSON serialization.
#[derive(Default)]
pub(super) struct ToolPreview {
    bytes: Vec<u8>,
    newlines: usize,
    truncated: bool,
}

impl ToolPreview {
    pub(super) fn push_str(&mut self, text: &str) {
        let _ = self.write_all(text.as_bytes());
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn contains(&self, text: &str) -> bool {
        self.text().contains(text)
    }

    fn text(&self) -> &str {
        // A serializer can split UTF-8 across writes; only the final prefix needs
        // to end on a character boundary.
        std::str::from_utf8(&self.bytes).unwrap_or_else(|error| {
            std::str::from_utf8(&self.bytes[..error.valid_up_to()]).unwrap()
        })
    }

    pub(super) fn finish(self) -> String {
        let mut text = self.text().to_owned();
        if self.truncated {
            text.push_str(TRUNCATION_NOTICE);
        }
        text
    }
}

impl Write for ToolPreview {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.truncated {
            return Err(io::Error::other("tool preview limit reached"));
        }
        let mut end = bytes.len().min(MAX_BYTES - self.bytes.len());
        for (index, byte) in bytes[..end].iter().enumerate() {
            if *byte == b'\n' {
                if self.newlines == MAX_LINES - 1 {
                    end = index;
                    break;
                }
                self.newlines += 1;
            }
        }
        self.bytes.extend_from_slice(&bytes[..end]);
        if end < bytes.len() {
            self.truncated = true;
            return Err(io::Error::other("tool preview limit reached"));
        }
        Ok(end)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "tool_preview_tests.rs"]
mod tests;
