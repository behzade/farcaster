use super::*;
use ropey::Rope;

#[test]
fn test_word_range() {
    let rope = Rope::from(concat!(
        "test text:\n",
        "abcde 中文🎉 test\n",
        "hello[()]\n",
        "test_connector ____\n",
        "Rope\n",
        "rök\n",
        "grande île\n",
    ));

    let tests = vec![
        (0, 0, Some("test")),
        (0, 4, Some(" ")),
        (1, 0, Some("abcde")),
        (1, 4, Some("abcde")),
        (1, 5, Some(" ")),
        (1, 6, Some("中")),
        (1, 9, Some("文")),
        (1, 13, Some("🎉")),
        (1, 20, Some("test")),
        (2, 5, Some("[")),
        (2, 6, Some("(")),
        (2, 7, Some(")")),
        (2, 8, Some("]")),
        (3, 5, Some("test_connector")),
        (3, 14, Some(" ")),
        (3, 16, Some("____")),
        (4, 0, Some("Rope")),
        (5, 0, Some("rök")),
        (6, 8, Some("île")),
    ];

    for (line, column, expected) in tests {
        let line_start_offset = rope.line_start_offset(line);
        let offset = line_start_offset + column;
        let range = TextSelector::word_range(&rope, offset);

        let actual = range.map(|r| rope.slice(r).to_string());
        let expect = expected.map(|s| s.to_string());
        assert_eq!(actual, expect, "line {}, column {}", line, column);
    }
}

#[test]
fn test_line_range() {
    let rope = Rope::from("first line\nsecond line\nthird");
    let tests = vec![
        (0, 0, "first line"),
        (0, 5, "first line"),
        (1, 3, "second line"),
        (2, 1, "third"),
    ];

    for (line, column, expected) in tests {
        let line_start_offset = rope.line_start_offset(line);
        let offset = line_start_offset + column;
        let range = TextSelector::line_range(&rope, offset);

        let actual = rope.slice(range).to_string();
        assert_eq!(actual, expected, "line {}, column {}", line, column);
    }
}
