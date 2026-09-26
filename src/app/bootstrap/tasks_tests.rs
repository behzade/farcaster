use super::*;
use futures::{executor::block_on, future};

#[test]
fn idle_tick_checks_for_notices_after_project_switch() {
    let (_send, updates) = async_channel::bounded(1);
    let mut refresh = NoticeRefresh { active: false };

    assert!(block_on(wait_for_notice_refresh(
        &updates,
        future::ready(()),
    )));
    assert!(!refresh.update(false));

    // A project switch does not post to the notice channel. The next tick
    // still checks the new project's board and redraws its existing notices.
    assert!(block_on(wait_for_notice_refresh(
        &updates,
        future::ready(()),
    )));
    assert!(refresh.update(true));
}

#[test]
fn final_expiry_redraws_once_then_empty_ticks_do_not() {
    let mut refresh = NoticeRefresh { active: true };

    assert!(refresh.update(true));
    assert!(refresh.update(false));
    assert!(!refresh.update(false));
}

#[test]
fn posted_notice_wakes_refresh_before_timer() {
    let (send, updates) = async_channel::bounded(1);
    send.try_send(()).expect("post wakes notice refresh");

    assert!(block_on(wait_for_notice_refresh(
        &updates,
        future::pending()
    )));
}

#[test]
fn closed_notice_updates_stop_refresh() {
    let (send, updates) = async_channel::bounded(1);
    drop(send);

    assert!(!block_on(wait_for_notice_refresh(
        &updates,
        future::pending(),
    )));
}
