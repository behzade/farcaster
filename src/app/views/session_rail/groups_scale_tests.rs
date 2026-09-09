use std::io::Write as _;
use std::{
    path::PathBuf,
    time::{Duration, Instant, UNIX_EPOCH},
};

use super::*;
use crate::sessions::{SessionSummary, UsageSummary};

/// A local benchmark for the archived-session rail at the current user's scale.
///
/// Run with `cargo test archived_rail_lists_792_large_roots -- --ignored --nocapture`.
/// The fixture contains synthetic metadata only and has no fixed timing threshold.
#[test]
#[ignore]
fn archived_rail_lists_792_large_roots() {
    const ROOTS: usize = 792;
    const SEARCH_BYTES: usize = 35 * 1024;

    let search = "x".repeat(SEARCH_BYTES);
    let sessions = (0..ROOTS)
        .map(|index| {
            SessionSummary::from_cached(
                format!("root-{index}"),
                PathBuf::from(format!("/synthetic/session-{index}")),
                PathBuf::from("/synthetic/project"),
                "Synthetic title".into(),
                "Synthetic preview".into(),
                String::new(),
                None,
                UNIX_EPOCH + Duration::from_secs(index as u64),
                1,
                UsageSummary::default(),
                true,
                false,
                search.clone(),
            )
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    let lists = session_rail_lists(&sessions, &[], None, &[]);
    let elapsed = started.elapsed();

    assert!(lists.active.is_empty());
    assert_eq!(lists.archived.len(), ROOTS);
    writeln!(
        std::io::stderr().lock(),
        "archived_rail_lists_792_large_roots elapsed_ms={:.2} roots={} search_mib={:.1}",
        elapsed.as_secs_f64() * 1_000.0,
        ROOTS,
        (ROOTS * SEARCH_BYTES) as f64 / (1024.0 * 1024.0),
    )
    .expect("write test diagnostics");
}
