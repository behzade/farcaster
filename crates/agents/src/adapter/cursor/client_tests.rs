use super::*;
use serde_json::json;

#[test]
fn connect_frames_require_a_trailer_and_preserve_errors() {
    let mut bytes = frame(br#"{}"#);
    bytes.extend(frame(br#"{"sdkMessage":{"type":"future-event"}}"#));
    let mut trailer = frame(br#"{"error":{"code":"unavailable","message":"lost stream"}}"#);
    trailer[0] = 2;
    bytes.extend(trailer);
    let mut stream = Stream {
        reader: Box::new(std::io::Cursor::new(bytes)),
        ended: false,
        timing: super::super::timing::Call::start("test.Send"),
        first_event: false,
    };
    assert_eq!(stream.next().expect("keepalive"), Some(json!({})));
    assert!(stream.next().expect("future event").is_some());
    assert!(
        stream
            .next()
            .expect_err("error trailer")
            .contains("lost stream")
    );
    let mut stream = Stream {
        reader: Box::new(std::io::Cursor::new(frame(br#"{}"#))),
        ended: false,
        timing: super::super::timing::Call::start("test.Send"),
        first_event: false,
    };
    stream.next().expect("message");
    assert!(stream.next().is_err(), "EOF is not successful completion");
}

#[test]
fn chunked_http_handles_split_frames_and_rejects_truncation() {
    let data = b"3\r\nabc\r\n2;extension=yes\r\nde\r\n0\r\nx-trailer: yes\r\n\r\n";
    let mut reader = Chunks {
        reader: std::io::Cursor::new(data),
        remaining: 0,
        ended: false,
    };
    let mut output = String::new();
    reader
        .read_to_string(&mut output)
        .expect("chunked response");
    assert_eq!(output, "abcde");
    let mut reader = Chunks {
        reader: std::io::Cursor::new(b"3\r\nab"),
        remaining: 0,
        ended: false,
    };
    assert!(reader.read_to_end(&mut Vec::new()).is_err());
}

#[test]
fn frame_lengths_flags_and_loopback_are_checked() {
    for bytes in [vec![1, 0, 0, 0, 0], vec![0, 255, 255, 255, 255]] {
        let mut stream = Stream {
            reader: Box::new(std::io::Cursor::new(bytes)),
            ended: false,
            timing: super::super::timing::Call::start("test.Send"),
            first_event: false,
        };
        assert!(stream.next().is_err());
    }
    assert!(Client::new("http://192.0.2.1:1234", "secret".into()).is_err());
    assert!(Client::new("http://127.0.0.1:1234", "secret\r\nBad: header".into()).is_err());
}
