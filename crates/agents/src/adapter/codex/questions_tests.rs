use super::*;

fn params() -> Value {
    json!({"threadId":"thread-1", "turnId":"turn-1", "isBlocking":false,
    "questions":[
        {"id":"choice", "header":"Choice", "question":"Which?", "options":[{"label":"A", "description":"First"}], "isOther":true},
        {"id":"token", "header":"Token", "question":"Token?", "options":null, "isSecret":true}
    ]})
}

#[test]
fn questions_preserve_flags_and_reply_by_question_id() {
    let pending = PendingQuestion::parse(CodexRequestId::String("7".into()), params()).unwrap();
    assert!(!pending.request.blocking);
    assert!(pending.request.questions[0].allow_free_text);
    assert!(pending.request.questions[1].secret);
    let response = QuestionResponse {
        id: pending.request.id.clone(),
        answers: BTreeMap::from([
            ("choice".into(), vec!["custom".into()]),
            ("token".into(), vec!["value".into()]),
        ]),
        cancelled: false,
    };
    assert_eq!(
        pending.reply(&response).unwrap(),
        json!({"answers":{
            "choice":{"answers":["custom"]}, "token":{"answers":["value"]}
        }})
    );
    assert_eq!(
        pending
            .reply(&QuestionResponse {
                cancelled: true,
                ..response
            })
            .unwrap(),
        json!({"answers":{}})
    );
}

#[test]
fn legacy_requests_block_and_incomplete_or_unknown_answers_are_rejected() {
    let mut value = params();
    value.as_object_mut().unwrap().remove("isBlocking");
    value["questions"][0]["isOther"] = json!(false);
    let pending = PendingQuestion::parse(CodexRequestId::Number(7), value).unwrap();
    assert!(pending.request.blocking);
    for answers in [
        BTreeMap::new(),
        BTreeMap::from([
            ("choice".into(), vec!["not an option".into()]),
            ("token".into(), vec!["value".into()]),
        ]),
        BTreeMap::from([
            ("unknown".into(), vec!["A".into()]),
            ("token".into(), vec!["value".into()]),
        ]),
    ] {
        assert!(
            pending
                .reply(&QuestionResponse {
                    id: "7".into(),
                    answers,
                    cancelled: false
                })
                .is_err()
        );
    }
}

#[test]
fn malformed_batches_are_rejected_without_echoing_content() {
    for questions in [
        json!([]),
        json!([
            {"id":"same", "header":"", "question":"secret"},
            {"id":"same", "header":"", "question":"secret"}
        ]),
        json!([{"id":"missing-fields"}]),
    ] {
        let mut value = params();
        value["questions"] = questions;
        let error = PendingQuestion::parse(CodexRequestId::Number(1), value)
            .err()
            .unwrap();
        assert!(!error.contains("secret"));
    }
}

#[test]
fn async_questions_keep_order_escape_answers_and_reject_malformed_batches() {
    let params = json!({"threadId":"thread", "turnId":"turn", "item":{
        "id":"message", "questions":[{"title":"First?", "options":["A","B"]}, {"title":"Second?"}]
    }});
    let pending = PendingQuestion::parse_async(&params).unwrap();
    let response = QuestionResponse {
        id: pending.request.id.clone(),
        answers: BTreeMap::from([
            ("0".into(), vec!["Custom \"answer\"".into()]),
            ("1".into(), vec!["Second\nline".into()]),
        ]),
        cancelled: false,
    };
    let text = pending.async_reply(&response).unwrap();
    assert_eq!(
        async_reply_display(&text).unwrap(),
        "> First?\n\nCustom \"answer\"\n\n> Second?\n\nSecond\nline"
    );
    assert!(
        pending
            .async_reply(&QuestionResponse {
                cancelled: true,
                ..response
            })
            .is_err()
    );
    assert!(
        async_reply_display(
            "<send_user_message_question_reply>broken</send_user_message_question_reply>"
        )
        .is_none()
    );
    for questions in [
        json!([]),
        json!([{"title":" "}]),
        json!([{"title":"Which?", "options":[]}]),
        json!([{"title":"Which?", "options":[" "]}]),
    ] {
        let mut malformed = params.clone();
        malformed["item"]["questions"] = questions;
        assert!(PendingQuestion::parse_async(&malformed).is_err());
    }
}
