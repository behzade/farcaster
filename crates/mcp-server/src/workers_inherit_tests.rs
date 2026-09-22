use super::*;
use crate::agents::HarnessAccessMode;

#[test]
fn inherited_children_snapshot_caller_and_keep_assignment_on_reuse() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let launches = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(Factory {
        launches: launches.clone(),
    });
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().to_owned(),
        2,
    )?;
    let caller = CallerRegistry::shared().issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Codex,
            provider: Some("openai".into()),
            model: Some("caller-model".into()),
            effort: Some("high".into()),
        },
        None,
        HarnessAccessMode::Full,
    );
    caller.bind("parent-session");
    let profiles = crate::agents::WorkerProfiles::default();
    let send = |name: &str, profile: Option<&str>| {
        super::send(
            &pool,
            SendParams {
                to: Some(name.into()),
                message: "inspect".into(),
                profile: profile.map(str::to_owned),
            },
            Some(caller.token().into()),
            &profiles,
            |_, _, mode| Some(mode),
        )
    };
    for (name, profile) in [("implicit", None), ("explicit", Some("inherit"))] {
        let result = send(name, profile)?;
        assert_eq!(result["created"], true);
        assert_eq!(
            result["assignment"],
            serde_json::json!({
                "profile": "inherit", "execution": {"harness": "codex-cli", "provider": "openai", "model": "caller-model", "effort": "high"}
            })
        );
    }
    caller.select_model("another-provider", "another-model");
    caller.set_effort(None);
    for profile in [None, Some("inherit")] {
        let result = send("implicit", profile)?;
        assert_eq!(result["created"], false);
        assert_eq!(result["assignment"]["execution"]["model"], "caller-model");
        assert_eq!(result["assignment"]["execution"]["effort"], "high");
    }
    assert!(send("implicit", Some("other")).is_err());
    wait_for_launches(&launches, 2)?;
    for launch in launches.lock().expect("launches").iter() {
        assert_eq!(launch.provider.as_deref(), Some("openai"));
        assert_eq!(launch.model.as_deref(), Some("caller-model"));
        assert_eq!(launch.effort.as_deref(), Some("high"));
    }
    let reused = send("implicit", Some("inherit"))?;
    assert_eq!(reused["created"], false);
    assert_eq!(reused["assignment"]["execution"]["provider"], "openai");
    assert_eq!(reused["assignment"]["execution"]["model"], "caller-model");
    Ok(())
}

#[test]
fn inheritance_fails_without_known_or_available_caller_model() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let factory: Arc<dyn WorkerSessionFactory> = Arc::new(Factory {
        launches: Arc::new(Mutex::new(Vec::new())),
    });
    let pool = WorkerPool::new(
        std::collections::BTreeMap::from([(Backend::Codex, factory)]),
        Backend::Codex,
        temp.path().into(),
        1,
    )?;
    let caller = CallerRegistry::shared().issue_with_access(
        temp.path(),
        CallerProfile {
            backend: Backend::Codex,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        HarnessAccessMode::Full,
    );
    caller.bind("parent-session");
    let send = || {
        super::send(
            &pool,
            SendParams {
                to: Some("child".into()),
                message: "inspect".into(),
                profile: None,
            },
            Some(caller.token().into()),
            &crate::agents::WorkerProfiles::default(),
            |_, _, _| None,
        )
    };
    assert!(
        send()
            .expect_err("unknown provider")
            .contains("caller provider is unknown")
    );
    caller.select_model("openai", "model");
    assert!(
        send()
            .expect_err("unavailable execution")
            .contains("unavailable")
    );
    Ok(())
}
