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
