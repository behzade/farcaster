use super::*;

#[test]
fn saved_model_refresh_and_launch_use_auto() {
    isolated(
        "access_mode_lifecycle_tests::saved_model_refresh_and_launch_use_auto",
        &["claude"],
        || {
            let harness = Harness::start();
            harness.select("claude", "auto");
            let saved: Model = serde_json::from_value(json!({
                "id":"gpt-5.6-sol", "name":"gpt-5.6-sol", "provider":"claude"
            }))
            .unwrap();
            harness
                .runtime
                .send(RuntimeCommand::SetModel(saved))
                .unwrap();
            let mut catalog = harness.accept(WAIT).expect("catalog starts");
            let init = catalog.request("initialize");
            let metadata = json!({
                "commands":[], "agents":[], "output_style":"default", "available_output_styles":[],
                "account":{}, "models":[{"value":"default", "resolvedModel":"gpt-5.6-sol",
                    "displayName":"Default", "description":"Test", "supportsAutoMode":true}]
            });
            catalog.reply(&init, metadata.clone());
            harness.snapshot("claude", |s| {
                s.access_mode == HarnessAccessMode::Auto
                    && s.available_access_modes()
                        .contains(&HarnessAccessMode::Auto)
                    && s.configuration_status == ConfigurationStatus::Loaded
            });
            harness
                .runtime
                .send(RuntimeCommand::SetAccessMode(HarnessAccessMode::Sandboxed))
                .unwrap();
            harness.snapshot("claude", |s| s.access_mode == HarnessAccessMode::Sandboxed);
            harness
                .runtime
                .send(RuntimeCommand::SetAccessMode(HarnessAccessMode::Auto))
                .unwrap();
            harness.snapshot("claude", |s| s.access_mode == HarnessAccessMode::Auto);
            harness
                .runtime
                .send(RuntimeCommand::Prompt {
                    target: "draft:auto".into(),
                    mode: PromptMode::Normal,
                    message: "test".into(),
                    display_message: None,
                    invocation: None,
                    images: vec![],
                    allow_while_running: false,
                })
                .unwrap();
            let mut session = harness.accept(WAIT).expect("session starts");
            let init = session.request("initialize");
            assert!(
                fs::read_to_string(harness.project.join("claude.args"))
                    .unwrap()
                    .lines()
                    .any(|arg| arg == "--permission-mode=auto")
            );
            session.reply(&init, metadata);
            let selection = session.request("set_model");
            session.reply(&selection, json!({}));
            let mut line = String::new();
            session.reader.read_line(&mut line).unwrap();
            let prompt: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(prompt["type"], "user");
            session.write(prompt);
            harness.snapshot("claude", |s| {
                s.connected && s.access_mode == HarnessAccessMode::Auto
            });
        },
    );
}
