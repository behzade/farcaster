use super::*;

#[test]
fn decodes_attachment_only_and_multiple_pastes() {
    let (text, files) = split_pasted_files(
        "Pasted text files:\n- [one.txt](</tmp/one.txt>)\n- [two.txt](</tmp/two.txt>)",
    );
    assert!(text.is_empty());
    assert_eq!(files.len(), 2);
    assert_eq!(files[1].path, PathBuf::from("/tmp/two.txt"));
}

#[test]
fn leaves_prose_and_malformed_lists_alone() {
    for text in [
        "See [one.txt](</tmp/one.txt>)",
        "Pasted text files:\n- ordinary prose",
        "Pasted text files:\n- [file](<https://example.com>)",
    ] {
        assert_eq!(split_pasted_files(text), (text, vec![]));
    }
}

#[test]
fn pasted_summary_finds_markers_across_fragment_boundaries() {
    for text in [
        "界\n\n--- BEGIN PASTED FILE f ---\npayload",
        "界\n\n\n--- BEGIN PASTED FILE f ---\npayload",
        "界\n\n--- BEGIN PASTED FILE f ---\n\n--- BEGIN PASTED FILE g ---",
        "界\n\n--- BEGIN PASTED FILX f ---\nordinary",
    ] {
        let expected = (pasted_file_summary(text).len() != text.len())
            .then(|| pasted_file_summary(text).len());
        for split in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            assert_eq!(
                pasted_file_summary_length([&text[..split], "", &text[split..]].into_iter()),
                expected
            );
        }
    }
}
