use std::sync::atomic::{AtomicU64, Ordering};

static SCANS: AtomicU64 = AtomicU64::new(0);
static PARSES: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CatalogMetrics {
    pub(crate) scans: u64,
    pub(crate) parses: u64,
    pub(crate) cache_hits: u64,
}

pub(in crate::modules::sessions) fn count_scan() {
    SCANS.fetch_add(1, Ordering::Relaxed);
}

pub(in crate::modules::sessions) fn count_parse() {
    PARSES.fetch_add(1, Ordering::Relaxed);
}

pub(in crate::modules::sessions) fn count_cache_hit() {
    CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn take_catalog_metrics() -> CatalogMetrics {
    CatalogMetrics {
        scans: SCANS.swap(0, Ordering::Relaxed),
        parses: PARSES.swap(0, Ordering::Relaxed),
        cache_hits: CACHE_HITS.swap(0, Ordering::Relaxed),
    }
}
