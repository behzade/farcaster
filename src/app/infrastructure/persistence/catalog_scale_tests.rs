use std::time::Instant;

use super::*;

/// A local benchmark for catalog loads at the current user's scale.
///
/// Run with `cargo test cached_catalog_decodes_1535_large_rows -- --ignored --nocapture`.
/// It uses synthetic metadata only and has no pass/fail timing threshold because debug and
/// release builds have different absolute timings.
#[test]
#[ignore]
fn cached_catalog_decodes_1535_large_rows() -> Result<(), String> {
    const SESSION_COUNT: usize = 1_535;
    const SEARCH_BYTES: usize = 52 * 1024;

    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let search = "x".repeat(SEARCH_BYTES);
    let sessions = (0..SESSION_COUNT)
        .map(|index| {
            SessionSummary::from_cached_for_harness(
                format!("session-{index}"),
                "codex-cli".into(),
                temp.path().join(format!("session-{index}")),
                PathBuf::from("/synthetic/project"),
                "Synthetic title".into(),
                "Synthetic preview".into(),
                "2026-09-09T00:00:00Z".into(),
                None,
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(index as u64),
                1,
                UsageSummary::default(),
                false,
                false,
                search.clone(),
            )
        })
        .collect::<Vec<_>>();
    let started = Instant::now();
    store.replace_sessions(&sessions)?;
    let import_elapsed = started.elapsed();

    let started = Instant::now();
    let sessions = store.cached_sessions("")?;
    let decode_elapsed = started.elapsed();

    assert_eq!(sessions.len(), SESSION_COUNT);
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.search_text().len())
            .sum::<usize>(),
        SESSION_COUNT * SEARCH_BYTES
    );
    let started = Instant::now();
    let cloned = sessions.clone();
    let clone_elapsed = started.elapsed();
    let started = Instant::now();
    let filtered = crate::sessions::filter_session_tree(cloned, "not-present");
    let filter_elapsed = started.elapsed();
    assert!(filtered.is_empty());

    // This is the unchanged-catalog comparison that runs on the UI thread after a refresh.
    let current_all = store.cached_sessions("")?;
    let current = crate::sessions::filter_session_tree(current_all.clone(), "");
    let next_all = store.cached_sessions("")?;
    let next = crate::sessions::filter_session_tree(next_all.clone(), "");
    let started = Instant::now();
    let catalog_changed = crate::app::change_detection::session_catalog_changed(
        &current,
        &current_all,
        None,
        &next,
        &next_all,
    );
    let comparison_elapsed = started.elapsed();
    assert!(!catalog_changed);
    eprintln!(
        "cached_catalog_decodes_1535_large_rows import_ms={:.2} decode_ms={:.2} clone_ms={:.2} miss_filter_ms={:.2} unchanged_ui_compare_ms={:.2} rows={} search_mib={:.1}",
        import_elapsed.as_secs_f64() * 1_000.0,
        decode_elapsed.as_secs_f64() * 1_000.0,
        clone_elapsed.as_secs_f64() * 1_000.0,
        filter_elapsed.as_secs_f64() * 1_000.0,
        comparison_elapsed.as_secs_f64() * 1_000.0,
        sessions.len(),
        (SESSION_COUNT * SEARCH_BYTES) as f64 / (1024.0 * 1024.0),
    );
    Ok(())
}
