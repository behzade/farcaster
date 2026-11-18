use super::*;
use serde_json::json;

#[test]
fn session_metrics_survive_reopen_and_partial_updates_without_crossing_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("state.sqlite3");
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    let metrics =
        json!({"contextUsage":{"tokens":120,"contextWindow":1000,"percent":12.0},"totalCost":0.5});
    let mut store = StateStore::open_at(&database).unwrap();
    for _ in 0..2 {
        store
            .save_session_metrics(Backend::Pi, directory.path(), &first, &metrics)
            .unwrap();
    }
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM session_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "unchanged facts need no duplicate event");
    store
        .save_session_metrics(
            Backend::Pi,
            directory.path(),
            &first,
            &json!({"totalCost":1.0}),
        )
        .unwrap();
    store
        .save_session_metrics(
            Backend::Pi,
            directory.path(),
            &first,
            &serde_json::Value::Null,
        )
        .unwrap();
    drop(store);

    let mut store = StateStore::open_at(&database).unwrap();
    let saved = store.load_session_metrics(Backend::Pi, &first).unwrap();
    assert_eq!(saved["contextUsage"], metrics["contextUsage"]);
    assert_eq!(saved["totalCost"], 1.0);
    assert_eq!(
        store.load_session_metrics(Backend::Pi, &second).unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        store.load_session_metrics(Backend::Claude, &first).unwrap(),
        serde_json::Value::Null
    );
    let lower = json!({"contextUsage":{"tokens":20,"contextWindow":1000,"percent":2.0}});
    store
        .save_session_metrics(Backend::Pi, directory.path(), &first, &lower)
        .unwrap();
    assert_eq!(
        store.load_session_metrics(Backend::Pi, &first).unwrap()["contextUsage"],
        lower["contextUsage"]
    );
}
