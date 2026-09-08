use super::*;

#[test]
fn child_questions_preserve_two_choices() {
    let request = child_interaction(agents::WorkerInput {
        id: "question".into(),
        prompt: "Choose".into(),
        options: vec!["First".into(), "Second".into()],
        secret: false,
    });
    assert!(matches!(request, ExtensionUiRequest::Select { options, .. }
            if options == ["First", "Second"]));
}
