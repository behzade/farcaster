use std::{path::PathBuf, sync::Arc, time::SystemTime};

use super::{SessionSummary, UsageSummary};

#[test]
fn cloned_session_shares_search_text() {
    let session = SessionSummary::from_cached(
        "session".into(),
        PathBuf::from("/session"),
        PathBuf::from("/project"),
        "Session".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        UsageSummary::default(),
        false,
        false,
        "search terms".into(),
    );

    let cloned = session.clone();

    assert!(Arc::ptr_eq(&session.search, &cloned.search));
    assert_eq!(cloned.search_text(), "search terms");
}
