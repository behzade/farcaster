use super::*;
use serde_json::json;
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

fn key(profile: &str, project: &str) -> Key {
    Key {
        harness: Backend::Cursor,
        profile: Some(profile.into()),
        project: project.into(),
        program: "fixture".into(),
        arguments: vec![],
        root: None,
        profile_directory: None,
    }
}
fn catalog() -> ConfigurationCatalog {
    serde_json::from_value(
        json!({"models":[{"id":"test","name":"Test","provider":"fixture",
        "adapterData":{"parameterName":"effort","request":{"model":"actual-id"}}}],"efforts":[]}),
    )
    .unwrap()
}

#[test]
fn concurrent_lookups_share_one_complete_catalog() {
    let cache = Cache::default();
    let calls = AtomicUsize::new(0);
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                let result = cache
                    .load(key("one", "/project"), || {
                        calls.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        Ok(catalog())
                    })
                    .unwrap();
                assert_eq!(result, catalog());
            });
        }
    });
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn persisted_catalog_preserves_request_data_and_scopes() {
    let cache = Cache::default();
    let restored = serde_json::from_slice(&serde_json::to_vec(&catalog()).unwrap()).unwrap();
    cache.seed(key("one", "/project"), restored);
    assert_eq!(
        cache.load(key("one", "/project"), || panic!("cached lookup fetched")),
        Ok(catalog())
    );
    assert!(
        cache
            .load(key("two", "/project"), || Err("different account".into()))
            .is_err()
    );
    assert!(
        cache
            .load(key("one", "/other"), || Err("different project".into()))
            .is_err()
    );
}

#[test]
fn failures_do_not_poison_later_lookups() {
    let cache = Cache::default();
    assert!(
        cache
            .load(key("one", "/project"), || Err("offline".into()))
            .is_err()
    );
    assert_eq!(
        cache.load(key("one", "/project"), || Ok(catalog())),
        Ok(catalog())
    );
}

#[test]
fn background_refresh_keeps_cached_requests_available_even_on_failure() {
    let cache = Cache::default();
    let scope = key("one", "/project");
    cache.seed(scope.clone(), catalog());
    let (entered, received) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let cache_ref = &cache;
        let refresh_scope = scope.clone();
        let refresh = threads.spawn(move || {
            cache_ref.lookup(refresh_scope, true, || {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                Err("offline".into())
            })
        });
        received.recv().unwrap();
        assert_eq!(
            cache.load(scope.clone(), || panic!("refresh blocked reuse")),
            Ok(catalog())
        );
        release.send(()).unwrap();
        assert!(refresh.join().unwrap().is_err());
    });
    assert_eq!(
        cache.load(scope.clone(), || panic!("failed refresh erased cache")),
        Ok(catalog())
    );
    let mut updated = catalog();
    updated.models[0].adapter_data = Some(json!({"request":{"model":"updated-id"}}));
    assert_eq!(
        cache.lookup(scope.clone(), true, || Ok(updated.clone())),
        Ok(updated.clone())
    );
    assert_eq!(
        cache.load(scope, || panic!("refreshed catalog not reused")),
        Ok(updated)
    );
}
