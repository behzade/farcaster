use super::*;

#[test]
fn arriving_requests_only_focus_when_replacing_the_composer_slot() {
    assert!(arriving_request_takes_focus(true));
    assert!(!arriving_request_takes_focus(false));
}

#[test]
fn activating_a_sheet_never_stacks_it_with_an_existing_sheet() {
    for sheet in [
        AppSheet::Sessions,
        AppSheet::Run,
        AppSheet::Keybindings,
        AppSheet::Settings,
        AppSheet::ProjectTrust,
    ] {
        let flags = sheet_flags(Some(sheet));
        assert_eq!(
            [
                flags.sessions,
                flags.run,
                flags.keybindings,
                flags.settings,
                flags.project_trust,
            ]
            .into_iter()
            .filter(|active| *active)
            .count(),
            1
        );
    }
    assert!(!sheet_flags(None).any());
}

#[test]
fn an_existing_sheet_prevents_recapturing_the_return_focus() {
    assert!(should_capture_return_focus(sheet_flags(None)));
    assert!(!should_capture_return_focus(sheet_flags(Some(
        AppSheet::Sessions
    ))));
    assert!(!should_capture_return_focus(sheet_flags(Some(
        AppSheet::Run
    ))));
}
