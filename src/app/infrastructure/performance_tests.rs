use super::*;

#[test]
fn operation_slots_follow_enum_discriminants() {
    for (index, kind) in OperationKind::ALL.into_iter().enumerate() {
        assert_eq!(kind as usize, index);
    }
}

#[test]
fn percentile_uses_the_nearest_rank() {
    let values = (1..=100).map(Duration::from_millis).collect::<Vec<_>>();
    assert_eq!(percentile(&values, 95), Duration::from_millis(95));
    assert_eq!(percentile(&[], 95), Duration::default());
}

#[test]
fn individual_timing_logs_only_slow_operations() {
    assert!(!should_log_duration(Duration::from_millis(1)));
    assert!(should_log_duration(Duration::from_millis(2)));
}
