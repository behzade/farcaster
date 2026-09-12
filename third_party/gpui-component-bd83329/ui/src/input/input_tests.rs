use super::*;
use crate::input::AnyInputState;

#[test]
fn content_types_map_to_accessibility_roles() {
    let cases = [
        (None, Role::TextInput),
        (Some(InputContentType::Name), Role::TextInput),
        (Some(InputContentType::NamePrefix), Role::TextInput),
        (Some(InputContentType::GivenName), Role::TextInput),
        (Some(InputContentType::MiddleName), Role::TextInput),
        (Some(InputContentType::FamilyName), Role::TextInput),
        (Some(InputContentType::NameSuffix), Role::TextInput),
        (Some(InputContentType::Nickname), Role::TextInput),
        (Some(InputContentType::JobTitle), Role::TextInput),
        (Some(InputContentType::OrganizationName), Role::TextInput),
        (Some(InputContentType::Location), Role::TextInput),
        (Some(InputContentType::FullStreetAddress), Role::TextInput),
        (Some(InputContentType::StreetAddressLine1), Role::TextInput),
        (Some(InputContentType::StreetAddressLine2), Role::TextInput),
        (Some(InputContentType::AddressCity), Role::TextInput),
        (Some(InputContentType::AddressState), Role::TextInput),
        (Some(InputContentType::AddressCityAndState), Role::TextInput),
        (Some(InputContentType::Sublocality), Role::TextInput),
        (Some(InputContentType::CountryName), Role::TextInput),
        (Some(InputContentType::PostalCode), Role::TextInput),
        (
            Some(InputContentType::TelephoneNumber),
            Role::PhoneNumberInput,
        ),
        (Some(InputContentType::EmailAddress), Role::EmailInput),
        (Some(InputContentType::Url), Role::UrlInput),
        (Some(InputContentType::CreditCardNumber), Role::TextInput),
        (Some(InputContentType::CreditCardName), Role::TextInput),
        (Some(InputContentType::CreditCardGivenName), Role::TextInput),
        (
            Some(InputContentType::CreditCardMiddleName),
            Role::TextInput,
        ),
        (
            Some(InputContentType::CreditCardFamilyName),
            Role::TextInput,
        ),
        (
            Some(InputContentType::CreditCardSecurityCode),
            Role::TextInput,
        ),
        (
            Some(InputContentType::CreditCardExpiration),
            Role::TextInput,
        ),
        (
            Some(InputContentType::CreditCardExpirationMonth),
            Role::TextInput,
        ),
        (
            Some(InputContentType::CreditCardExpirationYear),
            Role::TextInput,
        ),
        (Some(InputContentType::CreditCardType), Role::TextInput),
        (Some(InputContentType::Username), Role::TextInput),
        (Some(InputContentType::Password), Role::PasswordInput),
        (Some(InputContentType::NewPassword), Role::PasswordInput),
        (Some(InputContentType::OneTimeCode), Role::TextInput),
        (
            Some(InputContentType::ShipmentTrackingNumber),
            Role::TextInput,
        ),
        (Some(InputContentType::FlightNumber), Role::TextInput),
        (Some(InputContentType::DateTime), Role::DateTimeInput),
        (Some(InputContentType::Birthdate), Role::DateInput),
        (Some(InputContentType::BirthdateDay), Role::TextInput),
        (Some(InputContentType::BirthdateMonth), Role::TextInput),
        (Some(InputContentType::BirthdateYear), Role::TextInput),
        (Some(InputContentType::CellularEid), Role::TextInput),
        (Some(InputContentType::CellularImei), Role::TextInput),
    ];

    for (content_type, role) in cases {
        assert_eq!(
            accessibility_role(false, content_type, RoleOverride::Implicit),
            Some(role)
        );
    }
}

#[test]
fn multiline_inputs_keep_multiline_accessibility_role() {
    assert_eq!(
        accessibility_role(
            true,
            Some(InputContentType::Password),
            RoleOverride::Implicit
        ),
        Some(Role::MultilineTextInput)
    );
}

#[test]
fn explicit_accessibility_role_overrides_defaults() {
    assert_eq!(
        accessibility_role(
            false,
            Some(InputContentType::Password),
            Role::TextInput.into()
        ),
        Some(Role::TextInput)
    );
    assert_eq!(
        accessibility_role(
            true,
            Some(InputContentType::Password),
            Role::TextInput.into()
        ),
        Some(Role::TextInput)
    );
}

#[test]
fn presentational_role_emits_no_accessibility_node() {
    assert_eq!(
        accessibility_role(
            false,
            Some(InputContentType::Password),
            RoleOverride::Presentational
        ),
        None
    );
    assert_eq!(
        accessibility_role(true, None, RoleOverride::Presentational),
        None
    );
}

#[test]
fn role_option_converts_to_the_matching_override() {
    assert_eq!(
        RoleOverride::from(Some(Role::Button)),
        RoleOverride::Role(Role::Button)
    );
    assert_eq!(RoleOverride::from(None), RoleOverride::Presentational);
}

#[gpui::test]
fn editable_input_offers_accessibility_write_action(cx: &mut gpui::TestAppContext) {
    use crate::ElementExt as _;
    use gpui::{AppContext as _, Element as _, IntoElement as _, Render};
    use std::sync::{Arc, Mutex};

    type EmittedState = Option<(Option<String>, bool)>;

    struct InputA11yProbe {
        state: Entity<InputState>,
        emitted: Arc<Mutex<EmittedState>>,
    }

    impl Render for InputA11yProbe {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let state = self.state.clone();
            let emitted = self.emitted.clone();
            div().on_prepaint(move |_, window, cx| {
                let input = Input::new(&state).render(window, cx).into_element();
                let mut node = gpui::accesskit::Node::new(Role::TextInput);
                input.write_a11y_info(&mut node);
                *emitted.lock().unwrap() = Some((
                    node.value().map(ToOwned::to_owned),
                    node.supports_action(AccessibleAction::SetValue),
                ));
            })
        }
    }

    cx.update(crate::init);
    let emitted = Arc::new(Mutex::new(None));
    let captured = emitted.clone();
    let (probe, cx) = cx.add_window_view(move |window, cx| InputA11yProbe {
        state: cx.new(|cx| InputState::new(window, cx).default_value("initial")),
        emitted,
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    // No assistive technology is attached in tests, so the value stays
    // unmaterialized while `SetValue` is still advertised.
    assert_eq!(*captured.lock().unwrap(), Some((None, true)));

    let state = probe.read_with(cx, |probe, _| probe.state.clone());
    let base: TextInputState = state.clone().into();
    cx.update(|window, cx| {
        Input::handle_accessibility_set_value(&base, None, window, cx);
    });
    assert_eq!(state.read_with(cx, |state, _| state.value()), "initial");

    let action = gpui::accesskit::ActionData::Value("updated".into());
    cx.update(|window, cx| {
        Input::handle_accessibility_set_value(&base, Some(&action), window, cx);
    });
    assert_eq!(state.read_with(cx, |state, _| state.value()), "updated");
}

#[gpui::test]
fn input_emits_accessibility_id(cx: &mut gpui::TestAppContext) {
    use crate::ElementExt as _;
    use gpui::{AppContext as _, Element as _, IntoElement as _, Render};
    use std::sync::{Arc, Mutex};

    type EmittedIds = Vec<Option<String>>;

    struct InputA11yProbe {
        state: Entity<InputState>,
        emitted: Arc<Mutex<EmittedIds>>,
    }

    impl Render for InputA11yProbe {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let state = self.state.clone();
            let emitted = self.emitted.clone();
            div().on_prepaint(move |_, window, cx| {
                let mut author_id_of = |input: Input| {
                    let mut node = gpui::accesskit::Node::new(Role::TextInput);
                    input
                        .render(window, cx)
                        .into_element()
                        .write_a11y_info(&mut node);
                    node.author_id().map(ToOwned::to_owned)
                };

                *emitted.lock().unwrap() = vec![
                    author_id_of(Input::new(&state)),
                    author_id_of(Input::new(&state).accessibility_id("search.query")),
                ];
            })
        }
    }

    cx.update(crate::init);
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let captured = emitted.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| InputA11yProbe {
        state: cx.new(|cx| InputState::new(window, cx)),
        emitted,
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    assert_eq!(
        *captured.lock().unwrap(),
        vec![None, Some("search.query".into())]
    );
}

#[test]
fn accessibility_value_is_hidden_for_secret_inputs() {
    assert!(exposes_accessibility_value(false, None));
    assert!(!exposes_accessibility_value(true, None));
    assert!(!exposes_accessibility_value(
        false,
        Some(InputContentType::Password)
    ));
    assert!(!exposes_accessibility_value(
        false,
        Some(InputContentType::NewPassword)
    ));
}

#[gpui::test]
fn focused_input_registry_tracks_focus_and_blur(cx: &mut gpui::TestAppContext) {
    use crate::Root;
    use gpui::{AppContext as _, Render};

    struct Probe {
        input: Entity<InputState>,
        textarea: Entity<crate::input::TextareaState>,
        editor: Entity<crate::input::EditorState>,
        other: gpui::FocusHandle,
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div()
                .child(div().track_focus(&self.other))
                .child(Input::new(&self.input))
                .child(crate::input::Textarea::new(&self.textarea))
                .child(crate::input::Editor::new(&self.editor))
        }
    }

    cx.update(crate::init);
    let mut input = None;
    let mut textarea = None;
    let mut editor = None;
    let mut other_focus = None;
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let textarea_state = cx.new(|cx| crate::input::TextareaState::new(window, cx));
            let editor_state =
                cx.new(|cx| crate::input::EditorState::new(window, cx).language("rust"));
            input = Some(state.clone());
            textarea = Some(textarea_state.clone());
            editor = Some(editor_state.clone());
            let other = cx.focus_handle();
            other_focus = Some(other.clone());
            let probe = cx.new(|_| Probe {
                input: state,
                textarea: textarea_state,
                editor: editor_state,
                other,
            });
            cx.new(|cx| Root::new(probe, window, cx))
        })
        .unwrap()
    });
    let input = input.unwrap();
    let textarea = textarea.unwrap();
    let editor = editor.unwrap();
    let other_focus = other_focus.unwrap();
    let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);

    // Focusing each kind of input registers it, and blurring clears it.
    let cases: Vec<AnyInputState> = vec![
        input.clone().into(),
        textarea.clone().into(),
        editor.clone().into(),
    ];
    for expected in cases {
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| expected.focus_handle(cx).focus(window, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(
            cx.update(|window, cx| Root::read(window, cx).focused_input.clone()),
            Some(expected)
        );

        cx.update(|window, cx| other_focus.clone().focus(window, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(
            cx.update(|window, cx| Root::read(window, cx).focused_input.clone()),
            None
        );
    }
}
