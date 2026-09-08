use super::*;

#[test]
fn cancel_is_shown_only_without_a_refusal_choice() {
    for (label, show_cancel) in [
        ("Deny", false),
        ("Reject", false),
        ("Cancel", false),
        ("No", false),
        (" deny ", false),
        ("Other", true),
    ] {
        let request = ExtensionUiRequest::Select {
            id: "request".into(),
            title: "Run command?".into(),
            options: vec!["Accept".into(), label.into()],
            timeout: None,
        };
        assert_eq!(show_cancel_button(&request), show_cancel, "{label}");
    }
}

#[test]
fn confirmation_has_its_own_refusal() {
    let request = ExtensionUiRequest::Confirm {
        id: "request".into(),
        title: "Run command?".into(),
        message: String::new(),
        timeout: None,
    };
    assert!(!show_cancel_button(&request));
    assert_eq!(dialog_confirmation(&request, "n"), Some(("request", false)));
    assert_eq!(dialog_confirmation(&request, "y"), Some(("request", true)));
    for key in ["enter", "space", "1", "2", "escape"] {
        assert_eq!(dialog_confirmation(&request, key), None);
    }
    let input = ExtensionUiRequest::Input {
        id: "input".into(),
        title: "Explain".into(),
        placeholder: None,
        timeout: None,
    };
    assert_eq!(dialog_confirmation(&input, "y"), None);
}
