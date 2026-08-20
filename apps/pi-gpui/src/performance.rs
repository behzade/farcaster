//! DEBUG-only summaries from GPUI's frame, task, and action profilers.

use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use gpui::{FrameTimingCollector, TasksIncluded, WindowId, profiler};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const SLOW_OPERATION: Duration = Duration::from_millis(2);

static ENABLED: AtomicBool = AtomicBool::new(false);
static SNAPSHOTS_PUBLISHED: AtomicU64 = AtomicU64::new(0);
static STREAM_EVENTS_COALESCED: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPT_ITEMS_EXAMINED: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPT_ROWS_REMEASURED: AtomicU64 = AtomicU64::new(0);
static CATALOG_FILES_PARSED: AtomicU64 = AtomicU64::new(0);
static HIGHLIGHT_BYTES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn count_snapshot() {
    SNAPSHOTS_PUBLISHED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn count_coalesced_stream_event() {
    STREAM_EVENTS_COALESCED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn count_transcript_items(count: usize) {
    TRANSCRIPT_ITEMS_EXAMINED.fetch_add(count as u64, Ordering::Relaxed);
}

pub(crate) fn count_remeasured_rows(count: usize) {
    TRANSCRIPT_ROWS_REMEASURED.fetch_add(count as u64, Ordering::Relaxed);
}

pub(crate) fn count_catalog_parse() {
    CATALOG_FILES_PARSED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn count_highlight_bytes(count: usize) {
    HIGHLIGHT_BYTES.fetch_add(count as u64, Ordering::Relaxed);
}

#[must_use]
pub(crate) struct Timing {
    name: &'static str,
    started_at: Option<Instant>,
}

impl Timing {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            started_at: ENABLED.load(Ordering::Relaxed).then(Instant::now),
        }
    }

    pub(crate) fn cancel(mut self) {
        self.started_at = None;
    }
}

impl Drop for Timing {
    fn drop(&mut self) {
        let Some(started_at) = self.started_at else {
            return;
        };
        let elapsed = started_at.elapsed();
        if should_log_duration(elapsed) {
            zlog::info!(
                "PERF operation={} elapsed_ms={:.2}",
                self.name,
                elapsed.as_secs_f64() * 1_000.0
            );
        }
    }
}

pub(crate) struct PerformanceMonitor {
    frames: FrameTimingCollector,
    window_id: WindowId,
    sampled_at: Instant,
    pub(crate) summary: PerformanceSummary,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PerformanceSummary {
    pub(crate) sample_interval: Duration,
    pub(crate) frame_count: usize,
    pub(crate) draw_p95: Duration,
    pub(crate) draw_max: Duration,
    pub(crate) dirty_to_draw_p95: Duration,
    pub(crate) invalidations_average: f64,
    pub(crate) invalidations_max: u64,
    pub(crate) slowest_task: Option<String>,
    pub(crate) slowest_action: Option<String>,
    pub(crate) snapshots_published: u64,
    pub(crate) stream_events_coalesced: u64,
    pub(crate) transcript_items_examined: u64,
    pub(crate) transcript_rows_remeasured: u64,
    pub(crate) catalog_files_parsed: u64,
    pub(crate) highlight_bytes: u64,
}

impl PerformanceMonitor {
    pub(crate) fn new(window_id: WindowId) -> Self {
        ENABLED.store(true, Ordering::Relaxed);
        profiler::set_trace_enabled(true);
        profiler::set_frame_trace_enabled(true);
        Self {
            frames: FrameTimingCollector::new(),
            window_id,
            sampled_at: Instant::now(),
            summary: PerformanceSummary::default(),
        }
    }

    pub(crate) fn sample_if_due(&mut self) -> bool {
        let now = Instant::now();
        let sample_interval = now.duration_since(self.sampled_at);
        if sample_interval < SAMPLE_INTERVAL {
            return false;
        }
        self.sampled_at = now;
        self.summary = collect_summary(&mut self.frames, self.window_id, sample_interval);
        log_summary(&self.summary);
        true
    }
}

impl Drop for PerformanceMonitor {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::Relaxed);
        profiler::set_trace_enabled(false);
        profiler::set_frame_trace_enabled(false);
    }
}

fn collect_summary(
    frames: &mut FrameTimingCollector,
    window_id: WindowId,
    sample_interval: Duration,
) -> PerformanceSummary {
    let frames = frames
        .collect_unseen()
        .into_iter()
        .filter(|frame| frame.window_id == window_id)
        .collect::<Vec<_>>();
    let mut draw = frames
        .iter()
        .map(|frame| frame.draw_duration())
        .collect::<Vec<_>>();
    let mut dirty_to_draw = frames
        .iter()
        .filter_map(|frame| frame.dirty_to_draw_duration())
        .collect::<Vec<_>>();
    let invalidations = frames
        .iter()
        .map(|frame| frame.invalidations)
        .collect::<Vec<_>>();
    draw.sort_unstable();
    dirty_to_draw.sort_unstable();

    let slowest_task = profiler::take_all_stats(TasksIncluded::CompletedAndRunning)
        .into_iter()
        .flat_map(|thread| thread.stats.longest_poll_times)
        .max_by_key(|timing| timing.poll_duration())
        .filter(|timing| !timing.poll_duration().is_zero())
        .map(|timing| {
            format!(
                "{} · {}:{}",
                duration_label(timing.poll_duration()),
                timing.location.file(),
                timing.location.line()
            )
        });
    let slowest_action = profiler::take_action_stats()
        .longest_runtimes(true)
        .max_by_key(|timing| timing.runtime())
        .map(|timing| format!("{} · {}", duration_label(timing.runtime()), timing.name));

    PerformanceSummary {
        sample_interval,
        frame_count: frames.len(),
        draw_p95: percentile(&draw, 95),
        draw_max: draw.last().copied().unwrap_or_default(),
        dirty_to_draw_p95: percentile(&dirty_to_draw, 95),
        invalidations_average: if invalidations.is_empty() {
            0.0
        } else {
            invalidations.iter().sum::<u64>() as f64 / invalidations.len() as f64
        },
        invalidations_max: invalidations.into_iter().max().unwrap_or_default(),
        slowest_task,
        slowest_action,
        snapshots_published: SNAPSHOTS_PUBLISHED.swap(0, Ordering::Relaxed),
        stream_events_coalesced: STREAM_EVENTS_COALESCED.swap(0, Ordering::Relaxed),
        transcript_items_examined: TRANSCRIPT_ITEMS_EXAMINED.swap(0, Ordering::Relaxed),
        transcript_rows_remeasured: TRANSCRIPT_ROWS_REMEASURED.swap(0, Ordering::Relaxed),
        catalog_files_parsed: CATALOG_FILES_PARSED.swap(0, Ordering::Relaxed),
        highlight_bytes: HIGHLIGHT_BYTES.swap(0, Ordering::Relaxed),
    }
}

fn should_log_duration(duration: Duration) -> bool {
    duration >= SLOW_OPERATION
}

fn log_summary(summary: &PerformanceSummary) {
    zlog::info!(
        "PERF interval_ms={:.2} frames={} draw_p95_ms={:.2} draw_max_ms={:.2} dirty_to_draw_p95_ms={:.2} invalidations_avg={:.1} invalidations_max={} snapshots={} coalesced={} transcript_examined={} transcript_remeasured={} catalog_parses={} highlight_bytes={} slowest_task={:?} slowest_action={:?}",
        summary.sample_interval.as_secs_f64() * 1_000.0,
        summary.frame_count,
        summary.draw_p95.as_secs_f64() * 1_000.0,
        summary.draw_max.as_secs_f64() * 1_000.0,
        summary.dirty_to_draw_p95.as_secs_f64() * 1_000.0,
        summary.invalidations_average,
        summary.invalidations_max,
        summary.snapshots_published,
        summary.stream_events_coalesced,
        summary.transcript_items_examined,
        summary.transcript_rows_remeasured,
        summary.catalog_files_parsed,
        summary.highlight_bytes,
        summary.slowest_task,
        summary.slowest_action,
    );
}

fn percentile(values: &[Duration], percentile: usize) -> Duration {
    if values.is_empty() {
        return Duration::default();
    }
    let index = values
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[index]
}

pub(crate) fn duration_label(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
