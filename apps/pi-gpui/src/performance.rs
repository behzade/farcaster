//! DEBUG-only summaries from GPUI's frame, task, and action profilers.

use std::time::{Duration, Instant};

use gpui::{FrameTimingCollector, TasksIncluded, profiler};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct PerformanceMonitor {
    frames: FrameTimingCollector,
    sampled_at: Instant,
    pub(crate) summary: PerformanceSummary,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PerformanceSummary {
    pub(crate) frame_count: usize,
    pub(crate) draw_p95: Duration,
    pub(crate) draw_max: Duration,
    pub(crate) dirty_to_draw_p95: Duration,
    pub(crate) invalidations_average: f64,
    pub(crate) invalidations_max: u64,
    pub(crate) slowest_task: Option<String>,
    pub(crate) slowest_action: Option<String>,
}

impl PerformanceMonitor {
    pub(crate) fn new() -> Self {
        profiler::set_trace_enabled(true);
        profiler::set_frame_trace_enabled(true);
        Self {
            frames: FrameTimingCollector::new(),
            sampled_at: Instant::now(),
            summary: PerformanceSummary::default(),
        }
    }

    pub(crate) fn sample_if_due(&mut self) -> bool {
        if self.sampled_at.elapsed() < SAMPLE_INTERVAL {
            return false;
        }
        self.sampled_at = Instant::now();
        self.summary = collect_summary(&mut self.frames);
        true
    }
}

impl Drop for PerformanceMonitor {
    fn drop(&mut self) {
        profiler::set_trace_enabled(false);
        profiler::set_frame_trace_enabled(false);
    }
}

fn collect_summary(frames: &mut FrameTimingCollector) -> PerformanceSummary {
    let frames = frames.collect_unseen();
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
    }
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
}
