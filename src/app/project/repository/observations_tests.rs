use super::*;

fn observation(preference: BackendPreference, additions: u64) -> RepositoryObservation {
    RepositoryObservation {
        preference,
        backend: None,
        snapshot: None,
        additions: Some(additions),
        deletions: Some(1),
    }
}

#[test]
fn cache_evicts_the_least_recently_used_project() {
    let mut cache = ObservationCache::default();
    for index in 0..CACHE_CAPACITY {
        cache.remember(
            PathBuf::from(format!("/{index}")),
            observation(BackendPreference::Auto, index as u64),
        );
    }
    let first = PathBuf::from("/0");
    let reused = cache.reuse(&first, BackendPreference::Auto).unwrap();
    cache.remember(first.clone(), reused);
    cache.remember(
        PathBuf::from("/extra"),
        observation(BackendPreference::Auto, 99),
    );

    assert_eq!(cache.projects.len(), CACHE_CAPACITY);
    assert!(
        cache
            .reuse(Path::new("/1"), BackendPreference::Auto)
            .is_none()
    );
    assert_eq!(
        cache
            .reuse(&first, BackendPreference::Auto)
            .unwrap()
            .additions,
        Some(0)
    );
    assert!(cache.reuse(&first, BackendPreference::Auto).is_none());
}

#[test]
fn replacing_an_entry_updates_recency_without_using_another_slot() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    cache.remember(project.clone(), observation(BackendPreference::Auto, 1));
    cache.remember(project.clone(), observation(BackendPreference::Auto, 2));
    assert_eq!(cache.projects.len(), 1);
    assert_eq!(
        cache
            .reuse(&project, BackendPreference::Auto)
            .unwrap()
            .additions,
        Some(2)
    );
}

#[test]
fn invalidated_work_holds_the_slot_until_its_own_completion() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    let ticket = cache
        .begin(project.clone(), BackendPreference::Auto)
        .unwrap();
    cache.invalidate(&project);
    assert!(cache.busy());
    assert!(
        cache
            .begin(PathBuf::from("/other"), BackendPreference::Git)
            .is_none()
    );
    assert!(!cache.finish(&ticket));
    assert!(!cache.busy());

    let next = cache.begin(project, BackendPreference::Auto).unwrap();
    assert!(
        !cache.finish(&ticket),
        "a duplicate completion cannot release newer work"
    );
    assert!(cache.busy());
    assert!(cache.finish(&next));
    assert!(!cache.busy());
}

#[test]
fn old_scan_cannot_replace_a_newer_foreground_observation() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    let ticket = cache
        .begin(project.clone(), BackendPreference::Auto)
        .unwrap();
    cache.invalidate(&project); // Select the project while its scan is running.
    cache.remember(project.clone(), observation(BackendPreference::Auto, 42));
    assert!(!cache.finish(&ticket));
    assert_eq!(
        cache
            .reuse(&project, BackendPreference::Auto)
            .unwrap()
            .additions,
        Some(42)
    );
}

#[test]
fn remembering_or_reusing_a_project_invalidates_its_pending_scan() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    let first = cache
        .begin(project.clone(), BackendPreference::Auto)
        .unwrap();
    cache.remember(project.clone(), observation(BackendPreference::Auto, 42));
    assert!(!cache.finish(&first));
    let second = cache
        .begin(project.clone(), BackendPreference::Auto)
        .unwrap();
    assert!(cache.reuse(&project, BackendPreference::Auto).is_some());
    assert!(!cache.finish(&second));
}

#[test]
fn changing_backend_away_and_back_does_not_restore_an_old_ticket() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    let ticket = cache
        .begin(project.clone(), BackendPreference::Git)
        .unwrap();
    cache.remember(project.clone(), observation(BackendPreference::Jujutsu, 1));
    cache.remember(project.clone(), observation(BackendPreference::Git, 2));
    assert!(!cache.finish(&ticket));
    assert_eq!(
        cache
            .reuse(&project, BackendPreference::Git)
            .unwrap()
            .additions,
        Some(2)
    );
}

#[test]
fn mismatched_backend_drops_the_cached_observation() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    cache.remember(project.clone(), observation(BackendPreference::Git, 1));
    assert!(cache.reuse(&project, BackendPreference::Jujutsu).is_none());
    assert!(cache.reuse(&project, BackendPreference::Git).is_none());
}

#[test]
fn accepted_negative_scan_evicts_the_previous_observation() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    cache.remember(project.clone(), observation(BackendPreference::Auto, 1));
    let ticket = cache
        .begin(project.clone(), BackendPreference::Auto)
        .unwrap();
    assert!(cache.finish(&ticket));
    assert!(RepositoryObservation::from_scan(ticket.preference, Ok(None)).is_none());
    cache.forget(&project);
    assert!(cache.reuse(&project, BackendPreference::Auto).is_none());
}

#[test]
fn removing_projects_evicts_observations_and_invalidates_pending_work() {
    let mut cache = ObservationCache::default();
    let removed = PathBuf::from("/removed");
    let kept = PathBuf::from("/kept");
    cache.remember(removed.clone(), observation(BackendPreference::Auto, 1));
    cache.remember(kept.clone(), observation(BackendPreference::Auto, 2));
    let ticket = cache
        .begin(removed.clone(), BackendPreference::Auto)
        .unwrap();
    cache.retain(std::slice::from_ref(&kept));
    assert!(cache.busy());
    assert!(!cache.finish(&ticket));
    assert!(cache.reuse(&removed, BackendPreference::Auto).is_none());
    assert!(cache.reuse(&kept, BackendPreference::Auto).is_some());

    let ticket = cache.begin(kept.clone(), BackendPreference::Auto).unwrap();
    cache.forget(&kept);
    assert!(cache.busy());
    assert!(!cache.finish(&ticket));
}

#[test]
fn unrelated_changes_do_not_invalidate_pending_work() {
    let mut cache = ObservationCache::default();
    let project = PathBuf::from("/project");
    let ticket = cache
        .begin(project.clone(), BackendPreference::Git)
        .unwrap();
    cache.invalidate(Path::new("/other"));
    cache.remember(
        PathBuf::from("/other"),
        observation(BackendPreference::Auto, 1),
    );
    cache.retain(std::slice::from_ref(&project));
    assert!(cache.finish(&ticket));
}
