use super::*;

#[test]
fn session_setup_only_adds_requested_and_supported_context() {
    let options = SessionOptions {
        directories: vec!["/workspace/lib".into()],
        instructions: Some("Use the test fixtures".into()),
    };
    let init = json!({"agentCapabilities":{"sessionCapabilities":{"additionalDirectories":{}},"_meta":{"antigravity":{"systemPrompt":{"append":true}}}}});
    let mut params = json!({"cwd":"/workspace","mcpServers":[]});
    options.apply(&mut params, &init).expect("supported");
    assert_eq!(params["additionalDirectories"], json!(["/workspace/lib"]));
    assert_eq!(
        params.pointer("/_meta/antigravity/systemPrompt/append"),
        Some(&json!("Use the test fixtures"))
    );
    assert!(options.apply(&mut json!({}), &json!({})).is_err());
}
