use super::*;
use serde_json::json;

#[test]
fn unwraps_nested_results_and_preserves_order() {
    let image = json!({"type":"image","data":"AQID","mimeType":"image/png"});
    let content = vec![
        json!({"type":"text","text":"Before"}),
        json!({"type":"text","text":json!({"content":[image.clone()],"isError":false}).to_string()}),
        json!({"type":"resource_link","uri":"file:///tmp/a.html"}),
    ];
    assert_eq!(
        normalize_tool_output(&content).as_ref(),
        &[content[0].clone(), image, content[2].clone()]
    );
}

#[test]
fn borrows_plain_output_and_preserves_unknown_or_deep_wrappers() {
    for text in [
        "plain",
        "{",
        r#"{"content":[{"type":"unknown"}]}"#,
        r#"{"content":[{"type":"text","text":"hi"}],"count":1}"#,
    ] {
        let content = vec![json!({"type":"text","text":text})];
        assert!(matches!(normalize_tool_output(&content), Cow::Borrowed(_)));
    }
    let mut block = json!({"type":"image","data":"AQID","mimeType":"image/png"});
    for _ in 0..6 {
        block = json!({"type":"text","text":json!({"content":[block]}).to_string()});
    }
    let content = [block];
    assert_eq!(normalize_tool_output(&content)[0]["type"], "text");
}
