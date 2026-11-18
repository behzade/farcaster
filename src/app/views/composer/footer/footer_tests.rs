use super::*;

#[test]
fn context_hides_missing_usage_but_preserves_reported_zero() {
    let mut usage = ComposerUsage::default();
    assert_eq!(context_value(&usage), None);

    usage.context_percent = Some(0.0);
    assert_eq!(context_value(&usage), Some("0%".into()));

    usage.context_used = Some(0);
    usage.context_total = Some(200_000);
    assert_eq!(context_value(&usage), Some("0 / 200000 tokens · 0%".into()));

    usage.context_percent = None;
    usage.context_used = None;
    assert_eq!(context_value(&usage), None);

    usage.context_used = Some(25_000);
    usage.context_total = None;
    assert_eq!(context_value(&usage), Some("25000 tokens".into()));
}
