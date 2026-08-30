#[derive(Debug, Default)]
pub(crate) struct JsonlFramer {
    pending: Vec<u8>,
}

impl JsonlFramer {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut start = 0;
        for (index, byte) in self.pending.iter().enumerate() {
            if *byte == b'\n' {
                let mut end = index;
                if end > start && self.pending[end - 1] == b'\r' {
                    end -= 1;
                }
                frames.push(self.pending[start..end].to_vec());
                start = index + 1;
            }
        }
        if start > 0 {
            self.pending.drain(..start);
        }
        frames
    }

    /// Pi emits a final unterminated record at EOF, matching its reference reader.
    /// A trailing CR is payload unless it is immediately followed by LF.
    pub(crate) fn finish(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }
}

pub(crate) fn encode_json_line(value: &serde_json::Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::JsonlFramer;

    #[test]
    fn frames_arbitrary_chunks_and_multiple_records() {
        let mut framer = JsonlFramer::default();
        assert!(framer.push(b"{\"a\":").is_empty());
        assert_eq!(
            framer.push(b"1}\n{\"b\":2}\npartial"),
            vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]
        );
        assert_eq!(framer.finish(), Some(b"partial".to_vec()));
    }

    #[test]
    fn unicode_line_separators_are_payload_bytes() {
        let mut framer = JsonlFramer::default();
        let source = "{\"text\":\"a\u{2028}b\u{2029}c\"}\n";
        assert_eq!(
            framer.push(source.as_bytes()),
            vec![&source.as_bytes()[..source.len() - 1]]
        );
    }

    #[test]
    fn only_cr_adjacent_to_lf_is_stripped() {
        let mut framer = JsonlFramer::default();
        assert_eq!(
            framer.push(b"a\r\nb\rX\nc\r"),
            vec![b"a".to_vec(), b"b\rX".to_vec()]
        );
        assert_eq!(framer.finish(), Some(b"c\r".to_vec()));
    }

    #[test]
    fn unterminated_eof_preserves_trailing_cr_payload() {
        let mut framer = JsonlFramer::default();
        assert!(framer.push(b"payload\r").is_empty());
        assert_eq!(framer.finish(), Some(b"payload\r".to_vec()));
    }
}
