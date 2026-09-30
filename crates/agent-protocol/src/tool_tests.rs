use super::*;
use serde_json::json;

#[test]
fn borrowed_metadata_matching_follows_deserialization_defaults_and_validation() {
    let values = [
        json!({}),
        json!({"category":null,"title":null,"targets":[],"native":null}),
        json!({"category":"change","title":"界","targets":["a","b"],"native":{"id":1}}),
        json!({"category":"change","title":"界","targets":["b","a"],"native":{"id":1}}),
        json!({"category":"unknown_category"}),
        json!({"category":{"read":null}}),
        json!({"ignored":"extension"}),
        json!({"targets":null}),
        json!({"targets":[1]}),
        json!({"title":false}),
        json!({"category":12}),
        json!({"native":false}),
        json!({"native":[]}),
        json!([]),
        json!(["change"]),
        json!(["change","界",["a","b"],{"id":1}]),
        json!([null, null, [], null, "extra"]),
        json!([null, null, null]),
        Value::Null,
        json!("metadata"),
    ];
    let mut expected = vec![
        ToolMetadata::default(),
        ToolMetadata {
            native: Some(Value::Null),
            ..Default::default()
        },
    ];
    expected.extend(
        values
            .iter()
            .filter_map(|value| serde_json::from_value::<ToolMetadata>(value.clone()).ok()),
    );
    for value in values {
        let decoded = serde_json::from_value::<ToolMetadata>(value.clone());
        for metadata in &expected {
            assert_eq!(
                metadata.matches_value(&value),
                decoded.as_ref().is_ok_and(|value| value == metadata),
                "{value}, current={metadata:?}"
            );
        }
    }
}
