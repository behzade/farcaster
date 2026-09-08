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
#[path = "framing_tests.rs"]
mod tests;
