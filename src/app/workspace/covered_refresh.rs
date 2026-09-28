use std::time::Duration;

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_FRAME_POLLS: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum RefreshStep {
    Wait,
    Capture,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct CoveredRefresh {
    baseline: u64,
    polls: usize,
}

impl CoveredRefresh {
    pub(in crate::app) const fn new(baseline: u64) -> Self {
        Self { baseline, polls: 0 }
    }

    pub(in crate::app) const fn poll_interval() -> Duration {
        FRAME_POLL_INTERVAL
    }

    pub(in crate::app) fn observe(&mut self, frames: u64) -> RefreshStep {
        self.polls += 1;
        if frames > self.baseline || self.polls >= MAX_FRAME_POLLS {
            RefreshStep::Capture
        } else {
            RefreshStep::Wait
        }
    }
}

#[cfg(test)]
#[path = "covered_refresh_tests.rs"]
mod tests;
