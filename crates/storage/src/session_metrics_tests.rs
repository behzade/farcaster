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
    let facts = SessionFacts {
        metrics: metrics.clone(),
        goal: Some(agents::SessionGoal {
            objective: "Finish UI".into(),
            status: "active".into(),
            token_budget: Some(1000),
            tokens_used: 200,
            time_used_seconds: 20,
        }),
        outcome: Some(SessionOutcome::Failed),
        service_tier: Some("priority".into()),
        cache_usage: farcaster_agent_protocol::CacheUsage {
            read_tokens: 50,
            prompt_tokens: 200,
        },
    };
    let mut store = StateStore::open_at(&database).unwrap();
    for _ in 0..2 {
        store
            .save_session_facts(Backend::Pi, directory.path(), &first, &facts)
            .unwrap();
    }
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM session_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "unchanged facts need no duplicate event");
    store
        .save_session_facts(
            Backend::Pi,
            directory.path(),
            &first,
            &SessionFacts {
                metrics: json!({"totalCost":1.0}),
                ..facts.clone()
            },
        )
        .unwrap();
    store
        .save_session_facts(
            Backend::Pi,
            directory.path(),
            &first,
            &SessionFacts {
                metrics: serde_json::Value::Null,
                ..facts.clone()
            },
        )
        .unwrap();
    drop(store);

    let mut store = StateStore::open_at(&database).unwrap();
    let saved = store.load_session_facts(Backend::Pi, &first).unwrap();
    assert_eq!(saved.metrics["contextUsage"], metrics["contextUsage"]);
    assert_eq!(saved.metrics["totalCost"], 1.0);
    assert_eq!(
        store.load_session_facts(Backend::Pi, &second).unwrap(),
        SessionFacts::default()
    );
    assert_eq!(
        store.load_session_facts(Backend::Claude, &first).unwrap(),
        SessionFacts::default()
    );
    assert_eq!(saved.goal, facts.goal);
    assert_eq!(saved.outcome, facts.outcome);
    assert_eq!(saved.service_tier, facts.service_tier);
    assert_eq!(saved.cache_usage, facts.cache_usage);
    let lower = json!({"contextUsage":{"tokens":20,"contextWindow":1000,"percent":2.0}});
    store
        .save_session_facts(
            Backend::Pi,
            directory.path(),
            &first,
            &SessionFacts {
                metrics: lower.clone(),
                goal: None,
                ..saved
            },
        )
        .unwrap();
    assert_eq!(
        store
            .load_session_facts(Backend::Pi, &first)
            .unwrap()
            .metrics["contextUsage"],
        lower["contextUsage"]
    );
    assert!(
        store
            .load_session_facts(Backend::Pi, &first)
            .unwrap()
            .goal
            .is_none(),
        "an explicit goal clear survives reload"
    );
}

#[test]
fn legacy_metrics_restore_the_observed_tier_without_overriding_a_later_clear() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session");
    let mut store = StateStore::open_at(&directory.path().join("state.sqlite3")).unwrap();
    store
        .save_session_facts(
            Backend::Pi,
            directory.path(),
            &path,
            &SessionFacts::default(),
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE session_events SET body=json_remove(body,'$.serviceTier')",
            [],
        )
        .unwrap();
    store.connection.execute("INSERT INTO session_models(session_id,provider,model,effort,service_tier) SELECT id,'openai','model',NULL,'priority' FROM sessions", []).unwrap();
    assert_eq!(
        store
            .load_session_facts(Backend::Pi, &path)
            .unwrap()
            .service_tier
            .as_deref(),
        Some("priority")
    );
    store
        .save_session_facts(
            Backend::Pi,
            directory.path(),
            &path,
            &SessionFacts::default(),
        )
        .unwrap();
    assert_eq!(
        store
            .load_session_facts(Backend::Pi, &path)
            .unwrap()
            .service_tier,
        None
    );
}
