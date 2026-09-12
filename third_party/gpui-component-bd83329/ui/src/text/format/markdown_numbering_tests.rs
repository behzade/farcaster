use super::{NodeContext, parse};

#[test]
fn separate_ordered_lists_preserve_their_starting_numbers() {
    // Unindented paragraphs end each list, as in a response with commentary
    // between numbered recommendations.
    let source = "1. First recommendation\n\nCommentary.\n\n2. Second recommendation\n\nMore commentary.\n\n3. Third recommendation";
    let document = parse(source, &mut NodeContext::default()).unwrap();

    assert_eq!(
        document.to_markdown(),
        source,
        "separate ordered lists must not all restart at 1"
    );
}

#[test]
fn ordered_list_preserves_zero_start() {
    let source = "0. Zero\n1. One";
    let document = parse(source, &mut NodeContext::default()).unwrap();
    assert_eq!(document.to_markdown(), source);
}

#[test]
fn ordered_list_counts_from_start_not_each_source_marker() {
    let document = parse("9. Nine\n1. Ten\n1. Eleven", &mut NodeContext::default()).unwrap();
    assert_eq!(document.to_markdown(), "9. Nine\n10. Ten\n11. Eleven");
}
