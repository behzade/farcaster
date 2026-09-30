use super::*;

#[test]
fn file_totals_handle_nested_deleted_renamed_and_quoted_paths() {
    let patch = concat!(
        "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n---old\n+++new\n",
        "diff --git a/src/nested/deleted.rs b/src/nested/deleted.rs\n--- a/src/nested/deleted.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n",
        "diff --git a/quoted b/quoted\n--- /dev/null\n+++ \"b/src/\\303\\251\\tfile.rs\"\n@@ -0,0 +1 @@\n+hello\n",
    );
    let counts = parse(patch);
    for (path, expected) in [
        ("src/main.rs", (1, 1)),
        ("src/nested/deleted.rs", (0, 1)),
        ("new.rs", (0, 0)),
        ("src/é\tfile.rs", (1, 0)),
    ] {
        assert_eq!(counts.get(Path::new(path)), Some(&Some(expected)), "{path}");
    }
}

#[test]
fn untracked_files_count_their_lines_and_leave_binaries_unknown() {
    assert_eq!(
        untracked(b"".as_slice()).expect("count lines"),
        Some((0, 0))
    );
    assert_eq!(
        untracked(b"one\ntwo\n".as_slice()).expect("count lines"),
        Some((2, 0))
    );
    assert_eq!(
        untracked(b"one\ntwo".as_slice()).expect("count lines"),
        Some((2, 0))
    );
    assert_eq!(
        untracked(b"one\n\n".as_slice()).expect("count lines"),
        Some((2, 0))
    );
    assert_eq!(
        untracked(b"one\n\x00\n".as_slice()).expect("count lines"),
        None
    );

    let mut late_nul = vec![b'a'; BINARY_SNIFF_BYTES + 2];
    late_nul[BINARY_SNIFF_BYTES + 1] = b'\n';
    late_nul[BINARY_SNIFF_BYTES] = 0;
    assert_eq!(
        untracked(late_nul.as_slice()).expect("count lines"),
        Some((1, 0))
    );
}

#[test]
fn untracked_counts_preserve_buffer_and_binary_sniff_boundaries() {
    let mut contents = vec![b'x'; FILE_BUFFER_BYTES * 2 + 1];
    contents[FILE_BUFFER_BYTES - 1] = b'\n';
    contents[FILE_BUFFER_BYTES] = b'\n';
    contents[BINARY_SNIFF_BYTES] = 0;
    assert_eq!(
        untracked(contents.as_slice()).expect("count lines"),
        Some((3, 0))
    );
    contents[BINARY_SNIFF_BYTES - 1] = 0;
    let mut cursor = std::io::Cursor::new(&contents);
    assert_eq!(untracked(&mut cursor).expect("count lines"), None);
    assert_eq!(cursor.position(), contents.len() as u64, "read through EOF");
}

#[test]
fn untracked_retries_interrupted_reads_and_preserves_late_errors() {
    struct Reader(usize);
    impl Read for Reader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;
            match self.0 {
                1 => Err(io::ErrorKind::Interrupted.into()),
                2 => {
                    buffer[..2].copy_from_slice(b"\0x");
                    Ok(2)
                }
                _ => Err(io::Error::other("late read failure")),
            }
        }
    }
    let error = untracked(Reader(0)).expect_err("binary input still reads through EOF");
    assert_eq!(error.to_string(), "late read failure");
}
