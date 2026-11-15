use super::*;

#[test]
fn context_label_distinguishes_unavailable_zero_and_partial_data() {
    let mut usage = ComposerUsage::default();
    assert_eq!(context_value(&usage), "unavailable");

    usage.context_percent = Some(0.0);
    assert_eq!(context_value(&usage), "0%");

    usage.context_used = Some(0);
    usage.context_total = Some(200_000);
    assert_eq!(context_value(&usage), "0/200k");

    usage.context_percent = None;
    usage.context_used = None;
    assert_eq!(context_value(&usage), "200k capacity");

    usage.context_used = Some(25_000);
    usage.context_total = None;
    assert_eq!(context_value(&usage), "25.0k used");
}
