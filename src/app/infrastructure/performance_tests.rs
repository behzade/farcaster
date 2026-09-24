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

#[test]
fn tracing_every_operation_logs_phases_under_the_slow_operation_floor() {
    let instant = Duration::from_micros(1);
    assert!(!should_log_operation_with(instant, false));
    assert!(should_log_operation_with(instant, true));
    assert!(should_log_operation_with(SLOW_OPERATION, false));
}

#[test]
fn only_truthy_trace_values_force_every_operation_to_log() {
    for value in ["1", "true", "yes"] {
        assert!(trace_from_env(Some(value)));
    }
    for value in [None, Some(""), Some("0"), Some("false"), Some("off")] {
        assert!(!trace_from_env(value));
    }
}

#[test]
fn high_latency_requires_a_dropped_frame_or_long_render_queue() {
    let mut summary = PerformanceSummary {
        draw_max: HIGH_LATENCY_DRAW - Duration::from_millis(1),
        dirty_to_draw_p95: HIGH_LATENCY_DIRTY_TO_DRAW - Duration::from_millis(1),
        ..PerformanceSummary::default()
    };
    assert!(!is_high_latency(&summary));

    summary.draw_max = HIGH_LATENCY_DRAW;
    assert!(is_high_latency(&summary));

    summary.draw_max = Duration::default();
    summary.dirty_to_draw_p95 = HIGH_LATENCY_DIRTY_TO_DRAW;
    assert!(is_high_latency(&summary));
}

#[test]
fn high_latency_reports_are_rate_limited() {
    let summary = PerformanceSummary {
        draw_max: HIGH_LATENCY_DRAW,
        ..PerformanceSummary::default()
    };
    let now = Instant::now();
    assert!(should_report_high_latency(&summary, None, now));
    assert!(!should_report_high_latency(&summary, Some(now), now));
    assert!(should_report_high_latency(
        &summary,
        Some(now - HIGH_LATENCY_REPORT_COOLDOWN),
        now,
    ));
}
