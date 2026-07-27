use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::protocol::MAX_FRAME_BYTES;

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    Empty,
    TooLarge(usize),
    Json(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "frame I/O failed: {error}"),
            Self::Empty => write!(formatter, "empty frame"),
            Self::TooLarge(size) => {
                write!(formatter, "frame size {size} exceeds {MAX_FRAME_BYTES}")
            }
            Self::Json(error) => write!(formatter, "invalid frame JSON: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for FrameError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Reads one length-prefixed JSON frame, or `None` on clean EOF.
///
/// # Errors
///
/// Returns an error for partial, empty, oversized, malformed, or failed I/O.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<Option<T>, FrameError> {
    let mut length = [0_u8; 4];
    let first = match reader.read(&mut length[..1]) {
        Ok(0) => return Ok(None),
        Ok(read) => read,
        Err(error) => return Err(FrameError::Io(error)),
    };
    debug_assert_eq!(first, 1);
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 {
        return Err(FrameError::Empty);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Writes and flushes one length-prefixed JSON frame.
///
/// # Errors
///
/// Returns an error when JSON encoding or output fails, or the frame is too large.
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(value)?;
    if body.is_empty() {
        return Err(FrameError::Empty);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    let length = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(body.len()))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::protocol::{ClientRequest, ErrorCode, ServerEvent};

    use super::*;

    #[test]
    fn frames_do_not_depend_on_newlines() {
        let event = ServerEvent::Error {
            id: Some("one".to_owned()),
            code: ErrorCode::ProtocolError,
            message: "line one\nline two".to_owned(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &event).expect("write frame");
        let decoded = read_frame(&mut Cursor::new(bytes))
            .expect("read frame")
            .expect("one frame");
        assert_eq!(event, decoded);
    }

    #[test]
    fn rejects_oversized_frames_before_allocating_the_body() {
        let size = u32::try_from(MAX_FRAME_BYTES + 1).expect("test frame size");
        let error = read_frame::<ClientRequest>(&mut Cursor::new(size.to_be_bytes()))
            .expect_err("oversized frame must fail");
        assert!(matches!(error, FrameError::TooLarge(_)));
    }

    #[test]
    fn clean_eof_has_no_frame() {
        let frame =
            read_frame::<ClientRequest>(&mut Cursor::new(Vec::<u8>::new())).expect("clean EOF");
        assert_eq!(frame, None);
    }

    #[test]
    fn partial_length_is_a_protocol_error() {
        let error = read_frame::<ClientRequest>(&mut Cursor::new(vec![0_u8, 0]))
            .expect_err("partial frame length must fail");
        assert!(matches!(error, FrameError::Io(_)));
    }
}
