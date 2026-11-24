use super::*;

#[gpui::test]
fn current_model_stays_visible_without_catalog_match_and_reveals_resolved_alias(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::current_model_stays_visible_without_catalog_match_and_reveals_resolved_alias"
        ),
        cx,
        |cx, app, _, _| {
            let model = |provider: &str, id: &str| {
                serde_json::from_value::<crate::protocol::Model>(serde_json::json!({
                    "provider": provider, "id": id, "name": id,
                }))
                .unwrap()
            };
            let mut current = model("original", "current");
            current.service_tiers = vec!["standard".into(), "fast".into()];
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.snapshot = Arc::new(crate::runtime::RuntimeSnapshot {
                        prefill_model: Some(current),
                        models: vec![model("other", "current")],
                        ..Default::default()
                    });
                    app.set_runtime_picker_open(true, window, cx);
                });
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("runtime-current-model").is_some());
            assert!(cx.debug_bounds("runtime-model-current").is_none());
            assert!(cx.debug_bounds("runtime-service-tier-default").is_some());

            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_runtime_picker_open(false, window, cx);
                    let snapshot = Arc::make_mut(&mut app.snapshot);
                    snapshot.prefill_service_tier = Some("fast".into());
                    snapshot.models = (0..20)
                        .map(|index| model("original", &format!("model-{index}")))
                        .collect();
                    let mut alias = model("original", "catalog-alias");
                    alias.resolved_model = Some("current".into());
                    alias.service_tiers = vec!["standard".into(), "fast".into()];
                    snapshot.models.push(alias);
                    app.set_runtime_picker_open(true, window, cx);
                    assert_eq!(app.workspace.runtime_picker.highlighted, 20);
                });
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("runtime-current-model").is_none());
            assert!(cx.debug_bounds("runtime-service-tier-default").is_none());
            assert!(
                cx.debug_bounds("runtime-model-current").is_some(),
                "opening the picker must scroll the current catalog alias into view"
            );
        },
    );
}
