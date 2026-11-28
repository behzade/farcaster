use super::*;
use serde_json::json;

fn fixture(home: &Path, id: &str, images: &[&str]) -> Value {
    let path = home.join("rollout.jsonl");
    let output = images
        .iter()
        .map(|data| {
            json!({
                "type":"input_image", "image_url":format!("data:image/jpeg;base64,{data}")
            })
        })
        .collect::<Vec<_>>();
    let records = [
        json!({"type":"session_meta","payload":{"id":id}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","output":output}}),
    ];
    std::fs::write(
        &path,
        records.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    json!({"id":id,"path":path})
}

fn messages(data: &str) -> Vec<Value> {
    super::super::history_messages(&json!({
        "type":"mcpToolCall","id":"screenshot","status":"completed",
        "server":"computer","tool":"capture",
        "result":{"content":[{"type":"text","text":json!({"content":[
            {"type":"text","text":"Captured app"},
            {"type":"image","data":data,"mimeType":"image/png"}
        ]}).to_string()}]}
    }))
}

#[test]
fn restores_shortened_wrapped_images_without_changing_native_evidence() {
    let home = tempfile::tempdir().unwrap();
    let thread = fixture(
        home.path(),
        "thread",
        &["AQIDBAUGBwgJCgsM", "AQIDBAUGBwgJCgsM"],
    );
    let mut messages = messages("AQID…8 chars truncated…CgsM");
    let original_call = messages[0].clone();
    restore(&mut messages, &thread, home.path(), "thread");
    assert_eq!(messages[0], original_call, "retain native event evidence");
    assert_eq!(messages[1]["content"][0]["text"], "Captured app");
    assert_eq!(messages[1]["content"][1]["data"], "AQIDBAUGBwgJCgsM");
    assert_eq!(messages[1]["content"][1]["mimeType"], "image/jpeg");
    let mut conversation = farcaster_conversation::ConversationState::default();
    conversation.replace_history(&messages);
    assert_eq!(conversation.items[0].images.len(), 1);
    assert_eq!(
        conversation.items[0].images[0].bytes(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
}

#[test]
fn leaves_ambiguous_mismatched_and_intact_images_alone() {
    let home = tempfile::tempdir().unwrap();
    let thread = fixture(
        home.path(),
        "thread",
        &["AQIDBAUGBwgJCgsM", "AQIDAAAAAAAACgsM"],
    );
    for data in [
        "AQID…8 chars truncated…CgsM",
        "AQID…9 chars truncated…CgsM",
        "AQID",
        "not base64",
    ] {
        let mut messages = messages(data);
        let before = messages.clone();
        restore(&mut messages, &thread, home.path(), "thread");
        assert_eq!(messages, before);
    }
}

#[test]
fn restores_a_smaller_image_when_the_large_one_is_missing() {
    let home = tempfile::tempdir().unwrap();
    let thread = fixture(
        home.path(),
        "thread",
        &["AQID!AUGBwgJCgsM", "AQIDBAUGBwgJCgsM", "AQID?AUGBwgJCgsM"],
    );
    let large = "AQID…1000000 chars truncated…CgsM";
    let mut history = messages(large);
    history.extend(messages("AQID…8 chars truncated…CgsM"));
    restore(&mut history, &thread, home.path(), "thread");
    assert_eq!(history[1]["content"][1]["data"], large);
    assert_eq!(history[3]["content"][1]["data"], "AQIDBAUGBwgJCgsM");
}

#[test]
fn refuses_another_thread_or_profile_rollout() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let thread = fixture(home.path(), "thread", &["AQIDBAUGBwgJCgsM"]);
    for (profile, id) in [(home.path(), "wrong"), (other.path(), "thread")] {
        let mut messages = messages("AQID…8 chars truncated…CgsM");
        let before = messages.clone();
        restore(&mut messages, &thread, profile, id);
        assert_eq!(messages, before);
    }
}
