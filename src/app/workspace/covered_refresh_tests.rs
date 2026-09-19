use super::*;

#[test]
fn a_frame_drawn_after_the_change_captures_immediately() {
    let mut refresh = CoveredRefresh::new(7);
    assert_eq!(refresh.observe(7), RefreshStep::Wait);
    assert_eq!(refresh.observe(8), RefreshStep::Capture);
}

#[test]
fn frames_drawn_before_the_change_never_capture() {
    let mut refresh = CoveredRefresh::new(12);
    for frames in 0..=12 {
        assert_eq!(refresh.observe(frames), RefreshStep::Wait);
    }
    assert_eq!(refresh.observe(13), RefreshStep::Capture);
}

#[test]
fn a_surface_that_never_draws_still_captures_within_the_budget() {
    let mut refresh = CoveredRefresh::new(3);
    let mut waits = 0;
    loop {
        match refresh.observe(3) {
            RefreshStep::Wait => waits += 1,
            RefreshStep::Capture => break,
        }
        assert!(waits <= MAX_FRAME_POLLS);
    }
    assert_eq!(waits, MAX_FRAME_POLLS - 1);
    assert!(CoveredRefresh::poll_interval() <= Duration::from_millis(2));
}
