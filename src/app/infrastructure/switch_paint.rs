//! Per-selection timing for FARCASTER_PERF_TRACE. Keep only the first paint of
//! actual destination rows. A loading placeholder or the previous chat is excluded.
use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(in crate::app) enum SwitchPaintStage {
    ReaderStart,
    RepositoryReady,
    ReadDone,
    PagePrepared,
    Delivered,
    NavigationDone,
    ListLayout,
    Prepaint,
    Paint,
}

const STAGES: [&str; 9] = [
    "reader_start",
    "repository_ready",
    "read_done",
    "page_prepared",
    "delivered",
    "navigation_done",
    "layout",
    "prepaint",
    "paint_start",
];
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(in crate::app) struct SwitchPaintTrace {
    buffer: String,
    id: u64,
    started: Instant,
    times: [OnceLock<Duration>; STAGES.len()],
    result: OnceLock<(Duration, bool)>,
    #[cfg(test)]
    frame_geometry: OnceLock<std::sync::Mutex<Vec<Vec<(gpui::Pixels, gpui::Pixels)>>>>,
}

impl SwitchPaintTrace {
    pub(in crate::app) fn start(
        started: Instant,
        harness: crate::agents::Backend,
        path: &Path,
    ) -> Arc<Self> {
        Arc::new(Self {
            buffer: crate::sessions::session_buffer(harness, path),
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            started,
            times: Default::default(),
            result: OnceLock::new(),
            #[cfg(test)]
            frame_geometry: OnceLock::new(),
        })
    }

    pub(in crate::app) fn mark(&self, stage: SwitchPaintStage) {
        self.mark_at(stage, Instant::now());
    }

    pub(in crate::app) fn mark_at(&self, stage: SwitchPaintStage, at: Instant) {
        let _ = self.times[stage as usize].set(at.duration_since(self.started));
    }

    pub(in crate::app) fn active_for(&self, buffer: &str) -> bool {
        let collect = self.result.get().is_none();
        #[cfg(test)]
        let collect = collect || self.recording_geometry();
        collect && self.buffer == buffer
    }

    pub(in crate::app) fn painted(&self, before_ack: bool, rows: usize) {
        self.result.get_or_init(|| {
            let elapsed = self.started.elapsed();
            let mut phases = String::new();
            for (name, slot) in STAGES.iter().zip(&self.times) {
                if let Some(time) = slot.get() {
                    use std::fmt::Write as _;
                    let _ = write!(phases, " {name}_ms={:.3}", time.as_secs_f64() * 1_000.);
                }
            }
            zlog::info!("PERF operation=switch.first_paint trace={} elapsed_ms={:.3} rows={} before_ack={}{}", self.id, elapsed.as_secs_f64()*1_000., rows, before_ack, phases);
            (elapsed, before_ack)
        });
    }

    #[cfg(test)]
    pub(in crate::app) fn record_geometry(&self) {
        self.frame_geometry.get_or_init(Default::default);
    }

    #[cfg(test)]
    pub(in crate::app) fn recording_geometry(&self) -> bool {
        self.frame_geometry.get().is_some()
    }

    #[cfg(test)]
    pub(in crate::app) fn record_frame(&self, bounds: Vec<(gpui::Pixels, gpui::Pixels)>) {
        if let Some(frames) = self.frame_geometry.get() {
            frames.lock().unwrap().push(bounds);
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn assert_stable_frames(&self) -> usize {
        let frames = self.frame_geometry.get().unwrap().lock().unwrap();
        let first = frames.first().expect("destination never painted");
        assert!(!first.is_empty());
        for frame in &frames[1..] {
            assert_eq!(
                frame, first,
                "destination row bounds shifted between GPUI paints"
            );
        }
        frames.len()
    }

    #[cfg(test)]
    pub(in crate::app) fn result(&self) -> Option<(Duration, bool)> {
        self.result.get().copied()
    }
}
