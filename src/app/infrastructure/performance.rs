use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use gpui::{FrameTimingCollector, TasksIncluded, TouchPhase, WindowId, profiler};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const SLOW_OPERATION: Duration = Duration::from_millis(2);

static ENABLED: AtomicBool = AtomicBool::new(false);
static SNAPSHOTS_PUBLISHED: AtomicU64 = AtomicU64::new(0);
static STREAM_EVENTS_OBSERVED: AtomicU64 = AtomicU64::new(0);
static STREAM_EVENTS_COALESCED: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPT_ITEMS_COMPARED: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPT_ITEMS_PROJECTED: AtomicU64 = AtomicU64::new(0);
static TRANSCRIPT_ROWS_REMEASURED: AtomicU64 = AtomicU64::new(0);
static CATALOG_SCANS: AtomicU64 = AtomicU64::new(0);
static CATALOG_FILES_PARSED: AtomicU64 = AtomicU64::new(0);
static CATALOG_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static MARKDOWN_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static SCROLL_EVENTS: AtomicU64 = AtomicU64::new(0);
static SCROLL_STARTED: AtomicU64 = AtomicU64::new(0);
static SCROLL_MOVED: AtomicU64 = AtomicU64::new(0);
static SCROLL_ENDED: AtomicU64 = AtomicU64::new(0);
static SCROLL_EVENTS_AFTER_END: AtomicU64 = AtomicU64::new(0);
static SCROLL_AFTER_END_MAX_NS: AtomicU64 = AtomicU64::new(0);
static SCROLL_EVENT_GAP_MAX_NS: AtomicU64 = AtomicU64::new(0);
static SCROLL_DEFERRED_UPDATES: AtomicU64 = AtomicU64::new(0);
static SCROLL_DEFER_MAX_NS: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct ScrollTimingState {
    last_event: Option<Instant>,
    last_end: Option<Instant>,
}

thread_local! {
    static SCROLL_TIMING: RefCell<ScrollTimingState> = RefCell::default();
}

const OPERATION_COUNT: usize = 9;
static OPERATION_CALLS: [AtomicU64; OPERATION_COUNT] =
    [const { AtomicU64::new(0) }; OPERATION_COUNT];
static OPERATION_TOTAL_NS: [AtomicU64; OPERATION_COUNT] =
    [const { AtomicU64::new(0) }; OPERATION_COUNT];
static OPERATION_MAX_NS: [AtomicU64; OPERATION_COUNT] =
    [const { AtomicU64::new(0) }; OPERATION_COUNT];
static OPERATION_WORK: [AtomicU64; OPERATION_COUNT] =
    [const { AtomicU64::new(0) }; OPERATION_COUNT];

#[derive(Clone, Copy, Debug)]
pub(crate) enum OperationKind {
    TranscriptRow,
    MarkdownParse,
    ThinkingAssembly,
    FullProjection,
    ComposerHistory,
    FileMentionMatch,
    StateDatabase,
    HistoryLoad,
    RuntimeDrain,
}

impl OperationKind {
    const ALL: [Self; OPERATION_COUNT] = [
        Self::TranscriptRow,
        Self::MarkdownParse,
        Self::ThinkingAssembly,
        Self::FullProjection,
        Self::ComposerHistory,
        Self::FileMentionMatch,
        Self::StateDatabase,
        Self::HistoryLoad,
        Self::RuntimeDrain,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::TranscriptRow => "Transcript row layout",
            Self::MarkdownParse => "Markdown cache miss",
            Self::ThinkingAssembly => "Thinking text assembly",
            Self::FullProjection => "Full transcript projection",
            Self::ComposerHistory => "Composer history scan",
            Self::FileMentionMatch => "File mention match",
            Self::StateDatabase => "State database open/schema",
            Self::HistoryLoad => "Session history load",
            Self::RuntimeDrain => "Runtime event drain",
        }
    }

    const fn work_label(self) -> &'static str {
        match self {
            Self::TranscriptRow => "rows",
            Self::MarkdownParse => "bytes",
            Self::ThinkingAssembly => "chunks",
            Self::FullProjection | Self::ComposerHistory => "items",
            Self::FileMentionMatch => "files",
            Self::StateDatabase => "opens",
            Self::HistoryLoad => "entries",
            Self::RuntimeDrain => "events",
        }
    }
}

pub(crate) const fn sample_interval() -> Duration {
    SAMPLE_INTERVAL
}

fn add_counter(counter: &AtomicU64, count: u64) {
    if ENABLED.load(Ordering::Relaxed) {
        counter.fetch_add(count, Ordering::Relaxed);
    }
}

fn reset_counters() {
    let _ = crate::sessions::take_catalog_metrics();
    for counter in [
        &SNAPSHOTS_PUBLISHED,
        &STREAM_EVENTS_OBSERVED,
        &STREAM_EVENTS_COALESCED,
        &TRANSCRIPT_ITEMS_COMPARED,
        &TRANSCRIPT_ITEMS_PROJECTED,
        &TRANSCRIPT_ROWS_REMEASURED,
        &CATALOG_SCANS,
        &CATALOG_FILES_PARSED,
        &CATALOG_CACHE_HITS,
        &MARKDOWN_CACHE_HITS,
        &SCROLL_EVENTS,
        &SCROLL_STARTED,
        &SCROLL_MOVED,
        &SCROLL_ENDED,
        &SCROLL_EVENTS_AFTER_END,
        &SCROLL_AFTER_END_MAX_NS,
        &SCROLL_EVENT_GAP_MAX_NS,
        &SCROLL_DEFERRED_UPDATES,
        &SCROLL_DEFER_MAX_NS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
    SCROLL_TIMING.with(|state| *state.borrow_mut() = ScrollTimingState::default());
    for index in 0..OPERATION_COUNT {
        OPERATION_CALLS[index].store(0, Ordering::Relaxed);
        OPERATION_TOTAL_NS[index].store(0, Ordering::Relaxed);
        OPERATION_MAX_NS[index].store(0, Ordering::Relaxed);
        OPERATION_WORK[index].store(0, Ordering::Relaxed);
    }
}

pub(crate) fn count_snapshot() {
    add_counter(&SNAPSHOTS_PUBLISHED, 1);
}

pub(crate) fn count_stream_event(coalesced: bool) {
    add_counter(&STREAM_EVENTS_OBSERVED, 1);
    if coalesced {
        add_counter(&STREAM_EVENTS_COALESCED, 1);
    }
}

pub(crate) fn count_transcript_comparisons(count: usize) {
    add_counter(&TRANSCRIPT_ITEMS_COMPARED, count as u64);
}

pub(crate) fn count_transcript_projections(count: usize) {
    add_counter(&TRANSCRIPT_ITEMS_PROJECTED, count as u64);
}

pub(crate) fn count_remeasured_rows(count: usize) {
    add_counter(&TRANSCRIPT_ROWS_REMEASURED, count as u64);
}

pub(crate) fn record_catalog_metrics(metrics: crate::sessions::CatalogMetrics) {
    add_counter(&CATALOG_SCANS, metrics.scans);
    add_counter(&CATALOG_FILES_PARSED, metrics.parses);
    add_counter(&CATALOG_CACHE_HITS, metrics.cache_hits);
}

pub(crate) fn count_markdown_cache_hit() {
    add_counter(&MARKDOWN_CACHE_HITS, 1);
}

pub(crate) fn record_scroll_event(phase: TouchPhase) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let now = Instant::now();
    SCROLL_EVENTS.fetch_add(1, Ordering::Relaxed);
    match phase {
        TouchPhase::Started => {
            SCROLL_STARTED.fetch_add(1, Ordering::Relaxed);
            SCROLL_TIMING.with(|state| {
                let mut state = state.borrow_mut();
                state.last_event = None;
                state.last_end = None;
            });
        }
        TouchPhase::Moved => {
            SCROLL_MOVED.fetch_add(1, Ordering::Relaxed);
        }
        TouchPhase::Ended | TouchPhase::Cancelled => {
            SCROLL_ENDED.fetch_add(1, Ordering::Relaxed);
            SCROLL_TIMING.with(|state| state.borrow_mut().last_end = Some(now));
        }
    }
    SCROLL_TIMING.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(last_event) = state.last_event.replace(now) {
            SCROLL_EVENT_GAP_MAX_NS.fetch_max(
                duration_nanos(now.duration_since(last_event)),
                Ordering::Relaxed,
            );
        }
        if !matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
            && let Some(last_end) = state.last_end
        {
            SCROLL_EVENTS_AFTER_END.fetch_add(1, Ordering::Relaxed);
            SCROLL_AFTER_END_MAX_NS.fetch_max(
                duration_nanos(now.duration_since(last_end)),
                Ordering::Relaxed,
            );
        }
    });
}

pub(crate) fn record_scroll_defer(elapsed: Duration) {
    if ENABLED.load(Ordering::Relaxed) {
        SCROLL_DEFERRED_UPDATES.fetch_add(1, Ordering::Relaxed);
        SCROLL_DEFER_MAX_NS.fetch_max(duration_nanos(elapsed), Ordering::Relaxed);
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[must_use]
pub(crate) struct OperationTiming {
    kind: OperationKind,
    started_at: Option<Instant>,
    work: u64,
}

impl OperationTiming {
    pub(crate) fn new(kind: OperationKind, work: usize) -> Self {
        Self {
            kind,
            started_at: ENABLED.load(Ordering::Relaxed).then(Instant::now),
            work: work as u64,
        }
    }

    pub(crate) fn set_work(&mut self, work: usize) {
        self.work = work as u64;
    }

    pub(crate) fn increment_work(&mut self) {
        self.work = self.work.saturating_add(1);
    }
}

impl Drop for OperationTiming {
    fn drop(&mut self) {
        let Some(started_at) = self.started_at else {
            return;
        };
        let elapsed = started_at.elapsed();
        let elapsed_ns = duration_nanos(elapsed);
        let index = self.kind as usize;
        OPERATION_CALLS[index].fetch_add(1, Ordering::Relaxed);
        OPERATION_TOTAL_NS[index].fetch_add(elapsed_ns, Ordering::Relaxed);
        OPERATION_MAX_NS[index].fetch_max(elapsed_ns, Ordering::Relaxed);
        OPERATION_WORK[index].fetch_add(self.work, Ordering::Relaxed);
    }
}

/// Diagnostic wall-clock timing for startup work that happens before the
/// performance monitor is installed. Enabled by `DEBUG=true`.
#[must_use]
pub(crate) struct StartupTiming {
    name: &'static str,
    started_at: Option<Instant>,
}

impl StartupTiming {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            started_at: (std::env::var("DEBUG").ok().as_deref() == Some("true")).then(Instant::now),
        }
    }
}

impl Drop for StartupTiming {
    fn drop(&mut self) {
        let Some(started_at) = self.started_at else {
            return;
        };
        zlog::info!(
            "STARTUP operation={} elapsed_ms={:.2}",
            self.name,
            started_at.elapsed().as_secs_f64() * 1_000.0
        );
    }
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
    pub(crate) dirty_requests_average: f64,
    pub(crate) dirty_requests_max: u64,
    pub(crate) slowest_task: Option<String>,
    pub(crate) slowest_action: Option<String>,
    pub(crate) snapshots_published: u64,
    pub(crate) stream_events_observed: u64,
    pub(crate) stream_events_coalesced: u64,
    pub(crate) transcript_items_compared: u64,
    pub(crate) transcript_items_projected: u64,
    pub(crate) transcript_rows_remeasured: u64,
    pub(crate) catalog_scans: u64,
    pub(crate) catalog_files_parsed: u64,
    pub(crate) catalog_cache_hits: u64,
    pub(crate) markdown_cache_hits: u64,
    pub(crate) operations: Vec<OperationSummary>,
    pub(crate) scroll_events: u64,
    pub(crate) scroll_started: u64,
    pub(crate) scroll_moved: u64,
    pub(crate) scroll_ended: u64,
    pub(crate) scroll_events_after_end: u64,
    pub(crate) scroll_after_end_max: Duration,
    pub(crate) scroll_event_gap_max: Duration,
    pub(crate) scroll_deferred_updates: u64,
    pub(crate) scroll_defer_max: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OperationSummary {
    pub(crate) label: &'static str,
    pub(crate) calls: u64,
    pub(crate) total: Duration,
    pub(crate) max: Duration,
    pub(crate) work: u64,
    pub(crate) work_label: &'static str,
}

impl PerformanceMonitor {
    pub(crate) fn new(window_id: WindowId) -> Self {
        reset_counters();
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
    record_catalog_metrics(crate::sessions::take_catalog_metrics());
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
        dirty_requests_average: if invalidations.is_empty() {
            0.0
        } else {
            invalidations.iter().sum::<u64>() as f64 / invalidations.len() as f64
        },
        dirty_requests_max: invalidations.into_iter().max().unwrap_or_default(),
        slowest_task,
        slowest_action,
        snapshots_published: SNAPSHOTS_PUBLISHED.swap(0, Ordering::Relaxed),
        stream_events_observed: STREAM_EVENTS_OBSERVED.swap(0, Ordering::Relaxed),
        stream_events_coalesced: STREAM_EVENTS_COALESCED.swap(0, Ordering::Relaxed),
        transcript_items_compared: TRANSCRIPT_ITEMS_COMPARED.swap(0, Ordering::Relaxed),
        transcript_items_projected: TRANSCRIPT_ITEMS_PROJECTED.swap(0, Ordering::Relaxed),
        transcript_rows_remeasured: TRANSCRIPT_ROWS_REMEASURED.swap(0, Ordering::Relaxed),
        catalog_scans: CATALOG_SCANS.swap(0, Ordering::Relaxed),
        catalog_files_parsed: CATALOG_FILES_PARSED.swap(0, Ordering::Relaxed),
        catalog_cache_hits: CATALOG_CACHE_HITS.swap(0, Ordering::Relaxed),
        markdown_cache_hits: MARKDOWN_CACHE_HITS.swap(0, Ordering::Relaxed),
        scroll_events: SCROLL_EVENTS.swap(0, Ordering::Relaxed),
        scroll_started: SCROLL_STARTED.swap(0, Ordering::Relaxed),
        scroll_moved: SCROLL_MOVED.swap(0, Ordering::Relaxed),
        scroll_ended: SCROLL_ENDED.swap(0, Ordering::Relaxed),
        scroll_events_after_end: SCROLL_EVENTS_AFTER_END.swap(0, Ordering::Relaxed),
        scroll_after_end_max: Duration::from_nanos(
            SCROLL_AFTER_END_MAX_NS.swap(0, Ordering::Relaxed),
        ),
        scroll_event_gap_max: Duration::from_nanos(
            SCROLL_EVENT_GAP_MAX_NS.swap(0, Ordering::Relaxed),
        ),
        scroll_deferred_updates: SCROLL_DEFERRED_UPDATES.swap(0, Ordering::Relaxed),
        scroll_defer_max: Duration::from_nanos(SCROLL_DEFER_MAX_NS.swap(0, Ordering::Relaxed)),
        operations: OperationKind::ALL
            .into_iter()
            .map(|kind| {
                let index = kind as usize;
                OperationSummary {
                    label: kind.label(),
                    calls: OPERATION_CALLS[index].swap(0, Ordering::Relaxed),
                    total: Duration::from_nanos(
                        OPERATION_TOTAL_NS[index].swap(0, Ordering::Relaxed),
                    ),
                    max: Duration::from_nanos(OPERATION_MAX_NS[index].swap(0, Ordering::Relaxed)),
                    work: OPERATION_WORK[index].swap(0, Ordering::Relaxed),
                    work_label: kind.work_label(),
                }
            })
            .collect(),
    }
}

fn should_log_duration(duration: Duration) -> bool {
    duration >= SLOW_OPERATION
}

fn log_summary(summary: &PerformanceSummary) {
    let active_operations = summary
        .operations
        .iter()
        .filter(|operation| operation.calls > 0)
        .collect::<Vec<_>>();
    zlog::info!(
        "PERF interval_ms={:.2} frames={} draw_p95_ms={:.2} draw_max_ms={:.2} dirty_to_draw_p95_ms={:.2} dirty_requests_avg={:.1} dirty_requests_max={} snapshots={} stream_events={} stream_coalesced={} transcript_compared={} transcript_projected={} transcript_remeasured={} catalog_scans={} catalog_parses={} catalog_cache_hits={} markdown_cache_hits={} scroll_events={} scroll_phases={}/{}/{} scroll_after_end={} scroll_after_end_max_ms={:.2} scroll_gap_max_ms={:.2} scroll_defers={} scroll_defer_max_ms={:.2} operations={:?} slowest_task={:?} slowest_action={:?}",
        summary.sample_interval.as_secs_f64() * 1_000.0,
        summary.frame_count,
        summary.draw_p95.as_secs_f64() * 1_000.0,
        summary.draw_max.as_secs_f64() * 1_000.0,
        summary.dirty_to_draw_p95.as_secs_f64() * 1_000.0,
        summary.dirty_requests_average,
        summary.dirty_requests_max,
        summary.snapshots_published,
        summary.stream_events_observed,
        summary.stream_events_coalesced,
        summary.transcript_items_compared,
        summary.transcript_items_projected,
        summary.transcript_rows_remeasured,
        summary.catalog_scans,
        summary.catalog_files_parsed,
        summary.catalog_cache_hits,
        summary.markdown_cache_hits,
        summary.scroll_events,
        summary.scroll_started,
        summary.scroll_moved,
        summary.scroll_ended,
        summary.scroll_events_after_end,
        summary.scroll_after_end_max.as_secs_f64() * 1_000.0,
        summary.scroll_event_gap_max.as_secs_f64() * 1_000.0,
        summary.scroll_deferred_updates,
        summary.scroll_defer_max.as_secs_f64() * 1_000.0,
        active_operations,
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
}
