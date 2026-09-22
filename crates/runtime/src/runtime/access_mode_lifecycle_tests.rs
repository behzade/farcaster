use super::*;
use crate::agents::Backend;

#[test]
fn saved_model_refresh_and_launch_use_auto() {
    isolated(
        "access_mode_lifecycle_tests::saved_model_refresh_and_launch_use_auto",
        &["claude"],
        || {
            let harness = Harness::start();
            harness.select(Backend::Claude, "auto");
            let saved: Model = serde_json::from_value(json!({
                "id":"gpt-5.6-sol", "name":"gpt-5.6-sol", "provider":"claude"
            }))
            .expect("decode saved model");
            harness
                .runtime
                .send(RuntimeCommand::SetModel(saved))
                .expect("select saved model");
            let mut catalog = harness.accept(WAIT).expect("catalog starts");
            let init = catalog.request("initialize");
            let metadata = json!({
                "commands":[], "agents":[], "output_style":"default", "available_output_styles":[],
                "account":{}, "models":[{"value":"default", "resolvedModel":"gpt-5.6-sol",
                    "displayName":"Default", "description":"Test", "supportsAutoMode":true}]
            });
            catalog.reply(&init, metadata.clone());
            harness.snapshot(Backend::Claude, |s| {
                s.access_mode == HarnessAccessMode::Auto
                    && s.available_access_modes()
                        .contains(&HarnessAccessMode::Auto)
                    && s.configuration_status == ConfigurationStatus::Loaded
            });
            harness
                .runtime
                .send(RuntimeCommand::SetAccessMode(HarnessAccessMode::Sandboxed))
                .expect("select sandbox mode");
            harness.snapshot(Backend::Claude, |s| {
                s.access_mode == HarnessAccessMode::Sandboxed
            });
            harness
                .runtime
                .send(RuntimeCommand::SetAccessMode(HarnessAccessMode::Auto))
                .expect("select auto mode");
            harness.snapshot(Backend::Claude, |s| {
                s.access_mode == HarnessAccessMode::Auto
            });
            harness
                .runtime
                .send(RuntimeCommand::Prompt {
                    submission_id: "access-mode-prompt".into(),
                    target: "draft:auto".into(),
                    mode: PromptMode::Normal,
                    message: "test".into(),
                    display_message: None,
                    invocation: None,
                    images: vec![],
                    allow_while_running: false,
                })
                .expect("send runtime command");
            let mut session = harness.accept(WAIT).expect("session starts");
            let init = session.request("initialize");
            assert!(
                fs::read_to_string(harness.project.join("claude.args"))
                    .expect("read recorded arguments")
                    .lines()
                    .any(|arg| arg == "--permission-mode=auto")
            );
            session.reply(&init, metadata);
            let selection = session.request("set_model");
            session.reply(&selection, json!({}));
            let mut line = String::new();
            session.reader.read_line(&mut line).expect("read prompt");
            let prompt: Value = serde_json::from_str(&line).expect("decode prompt");
            assert_eq!(prompt["type"], "user");
            session.write(prompt);
            harness.snapshot(Backend::Claude, |s| {
                s.connected && s.access_mode == HarnessAccessMode::Auto
            });
        },
    );
}
