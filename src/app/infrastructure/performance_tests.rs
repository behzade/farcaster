use super::*;

#[gpui::test]
fn basic_monitor_reads_each_draw_once_without_full_tracing(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let id = window.window_handle().window_id();
        let mut monitor = PerformanceMonitor::new(id, false, cx);
        assert!(!profiler::trace_enabled());
        window.refresh();
        window.draw(cx).clear(cx);
        let summary = collect_summary(&mut monitor.frames, id, SAMPLE_INTERVAL, false);
        assert_eq!(summary.frame_count, 1);
        assert!(summary.draw_max > Duration::ZERO);
        assert!(summary.dirty_requests_max > 0);
        assert_eq!(
            collect_summary(&mut monitor.frames, id, SAMPLE_INTERVAL, false).frame_count,
            0
        );
        profiler::set_trace_enabled(true);
        window.refresh();
        window.draw(cx).clear(cx);
        profiler::set_trace_enabled(false);
        assert_eq!(
            collect_summary(&mut monitor.frames, id, SAMPLE_INTERVAL, false).frame_count,
            1
        );
    });
}

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
