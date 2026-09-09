use super::{bindings, platform_key, registry};

#[test]
fn session_numbers_work_in_embedded_views_without_claiming_control_keys() {
    use crate::app::{APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT};

    for (prefix, platform) in [("cmd", "cmd"), ("ctrl", "super")] {
        let keymap = gpui::Keymap::new(
            super::registry_for_platform(prefix)
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        for number in 0..=9 {
            for context in [APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT] {
                let contexts =
                    [gpui::KeyContext::parse(context).expect("test operation should succeed")];
                let (bindings, _) = keymap.bindings_for_input(
                    &[gpui::Keystroke::parse(&format!("{platform}-{number}"))
                        .expect("test operation should succeed")],
                    &contexts,
                );
                assert_eq!(bindings.len(), 1, "{platform}-{number} in {context}");
                let action = bindings[0].action().name();
                assert!(action.ends_with(&format!("SwitchSession{number}")));
                let (aliases, _) = keymap.bindings_for_input(
                    &[gpui::Keystroke::parse(&format!("ctrl-{number}"))
                        .expect("test operation should succeed")],
                    &contexts,
                );
                if context == APP_INPUT_CONTEXT {
                    assert_eq!(aliases.len(), 1);
                    assert_eq!(aliases[0].action().name(), action);
                } else {
                    assert!(aliases.is_empty(), "ctrl-{number} in {context}");
                }
            }
        }
    }
}

#[test]
fn root_focus_traversal_is_unbound() {
    let bindings = bindings();
    let root_context = gpui::KeyBindingContextPredicate::parse("Root").expect("root context");
    for (keystroke, target) in [("tab", "root::Tab"), ("shift-tab", "root::TabPrev")] {
        assert!(bindings.iter().any(|binding| {
            binding
                .action()
                .as_any()
                .downcast_ref::<gpui::Unbind>()
                .is_some_and(|unbind| unbind.0.as_ref() == target)
                && binding
                    .match_keystrokes(&[gpui::Keystroke::parse(keystroke).expect("test keystroke")])
                    == Some(false)
                && binding.predicate().as_deref() == Some(&root_context)
        }));
    }
}

#[test]
fn picker_shortcuts_route_only_in_their_owned_contexts() {
    use super::registry_for_platform;
    use crate::app::{
        APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT, RestoreSession, SetRuntime, SetSandbox,
        ShowEditor, ShowTerminal,
    };

    for prefix in ["cmd", "ctrl"] {
        let keymap = gpui::Keymap::new(
            registry_for_platform(prefix)
                .into_iter()
                .map(|shortcut| shortcut.binding)
                .collect(),
        );
        for (suffix, action) in [
            ("shift-s", Box::new(SetSandbox) as Box<dyn gpui::Action>),
            ("shift-m", Box::new(SetRuntime) as Box<dyn gpui::Action>),
            ("shift-a", Box::new(RestoreSession) as Box<dyn gpui::Action>),
            ("e", Box::new(ShowEditor) as Box<dyn gpui::Action>),
            ("t", Box::new(ShowTerminal) as Box<dyn gpui::Action>),
            (
                "shift-p",
                Box::new(crate::app::ShowActionPicker) as Box<dyn gpui::Action>,
            ),
        ] {
            let stroke = gpui::Keystroke::parse(&format!("{prefix}-{suffix}"))
                .expect("test operation should succeed");
            let (bindings, _) = keymap.bindings_for_input(
                std::slice::from_ref(&stroke),
                &[gpui::KeyContext::parse(APP_INPUT_CONTEXT)
                    .expect("test operation should succeed")],
            );
            assert_eq!(
                bindings
                    .first()
                    .expect("test operation should succeed")
                    .action()
                    .name(),
                action.name()
            );
            let (bindings, _) = keymap.bindings_for_input(
                &[stroke],
                &[gpui::KeyContext::parse(NATIVE_INPUT_CONTEXT)
                    .expect("test operation should succeed")],
            );
            assert!(bindings.is_empty());
        }
        for suffix in ["m", "l"] {
            let key = format!("{prefix}-{suffix}");
            let (bindings, _) = keymap.bindings_for_input(
                &[gpui::Keystroke::parse(&key).expect("test operation should succeed")],
                &[gpui::KeyContext::parse(APP_INPUT_CONTEXT)
                    .expect("test operation should succeed")],
            );
            assert!(bindings.is_empty(), "{key} must remain unbound");
        }
        for (context, expected) in [
            ("PiPicker", true),
            (APP_INPUT_CONTEXT, false),
            (NATIVE_INPUT_CONTEXT, false),
        ] {
            let (bindings, _) = keymap.bindings_for_input(
                &[gpui::Keystroke::parse("alt-left").expect("test operation should succeed")],
                &[
                    gpui::KeyContext::parse(context).expect("test operation should succeed"),
                    gpui::KeyContext::parse("Input").expect("test operation should succeed"),
                ],
            );
            assert_eq!(
                bindings.first().is_some_and(|binding| binding
                    .action()
                    .as_any()
                    .is::<crate::app::PickerNavigateBack>()),
                expected
            );
        }
    }
}

#[test]
fn tab_navigation_stays_in_picker_input() {
    let keymap = gpui::Keymap::new(bindings());
    for (key, action) in [
        ("tab", &super::SelectDown as &dyn gpui::Action),
        ("shift-tab", &super::SelectUp as &dyn gpui::Action),
    ] {
        for context in [
            "PiPicker",
            "FarcasterComposer",
            crate::app::NATIVE_INPUT_CONTEXT,
        ] {
            let (bindings, _) = keymap.bindings_for_input(
                &[gpui::Keystroke::parse(key).expect("test operation should succeed")],
                &[
                    gpui::KeyContext::parse("Root").expect("test operation should succeed"),
                    gpui::KeyContext::parse(context).expect("test operation should succeed"),
                    gpui::KeyContext::parse("Input").expect("test operation should succeed"),
                ],
            );
            assert_eq!(
                bindings
                    .first()
                    .is_some_and(|binding| binding.action().name() == action.name()),
                context == "PiPicker",
                "{key} in {context}",
            );
        }
    }
}

#[test]
fn composer_completion_keys_require_visible_suggestions() {
    use super::registry_for_platform;
    use crate::app::{
        ComposerCompletionNext, ComposerCompletionPrevious, NewSession, SubmitFollowUp,
    };
    use gpui::Action as _;

    let keymap = gpui::Keymap::new(
        registry_for_platform("ctrl")
            .into_iter()
            .map(|shortcut| shortcut.binding)
            .collect(),
    );
    for open in [false, true] {
        let contexts = [
            gpui::KeyContext::parse(crate::app::APP_INPUT_CONTEXT)
                .expect("test operation should succeed"),
            gpui::KeyContext::parse(if open {
                "FarcasterComposer Completions"
            } else {
                "FarcasterComposer"
            })
            .expect("test operation should succeed"),
            gpui::KeyContext::parse("Input").expect("test operation should succeed"),
        ];
        for (key, completion) in [
            ("ctrl-n", ComposerCompletionNext.name()),
            ("ctrl-p", ComposerCompletionPrevious.name()),
            ("tab", ComposerCompletionNext.name()),
            ("shift-tab", ComposerCompletionPrevious.name()),
        ] {
            let (bindings, _) = keymap.bindings_for_input(
                &[gpui::Keystroke::parse(key).expect("test operation should succeed")],
                &contexts,
            );
            let expected = if open {
                Some(completion)
            } else if key == "ctrl-n" {
                Some(NewSession.name())
            } else if key == "tab" {
                Some(SubmitFollowUp.name())
            } else {
                None
            };
            assert_eq!(
                bindings.first().map(|binding| binding.action().name()),
                expected,
                "{key}, suggestions open: {open}"
            );
        }
    }
}

#[test]
fn legacy_global_actions_are_scoped_to_app_owned_contexts() {
    use crate::app::APP_SHORTCUT_CONTEXT;
    let app_context = gpui::KeyBindingContextPredicate::parse(APP_SHORTCUT_CONTEXT)
        .expect("app shortcut context");
    let shortcuts = registry();
    for keystroke in ["f1", "f2", "f3", "f4", "ctrl-tab", "ctrl-shift-tab"] {
        let matches = shortcuts
            .iter()
            .filter(|shortcut| shortcut.keystroke == keystroke)
            .collect::<Vec<_>>();
        assert!(!matches.is_empty(), "{keystroke} must remain registered");
        for shortcut in matches {
            assert_eq!(
                shortcut.binding.predicate().as_deref(),
                Some(&app_context),
                "{keystroke} must not be a global None-context binding"
            );
        }
    }
}

#[test]
fn application_shortcuts_stay_in_app_owned_contexts() {
    use super::registry_for_platform;
    use crate::app::{APP_INPUT_CONTEXT, NATIVE_INPUT_CONTEXT};

    let keymap = gpui::Keymap::new(
        registry_for_platform("cmd")
            .into_iter()
            .map(|shortcut| shortcut.binding)
            .collect(),
    );
    let app_contexts =
        [gpui::KeyContext::parse(APP_INPUT_CONTEXT).expect("test operation should succeed")];
    let native_contexts =
        [gpui::KeyContext::parse(NATIVE_INPUT_CONTEXT).expect("test operation should succeed")];
    for key in ["cmd-n", "cmd-e", "cmd-t", "cmd-k", "cmd-g"] {
        if key == "cmd-g" && !cfg!(target_os = "macos") {
            continue;
        }
        let stroke = gpui::Keystroke::parse(key).expect("test operation should succeed");
        let (app_bindings, _) =
            keymap.bindings_for_input(std::slice::from_ref(&stroke), &app_contexts);
        assert!(!app_bindings.is_empty(), "{key} missing in app context");
        let (native_bindings, _) = keymap.bindings_for_input(&[stroke], &native_contexts);
        assert!(
            native_bindings.is_empty(),
            "{key} must not reach embedded views"
        );
    }
    let (ctrl_j, _) = keymap.bindings_for_input(
        &[gpui::Keystroke::parse("ctrl-j").expect("test operation should succeed")],
        &app_contexts,
    );
    assert!(ctrl_j.is_empty(), "Ctrl+J must not open the terminal");
    for key in ["f1", "f2", "f3", "f4", "ctrl-tab", "ctrl-shift-tab"] {
        let stroke = gpui::Keystroke::parse(key).expect("test operation should succeed");
        let (native_bindings, _) =
            keymap.bindings_for_input(std::slice::from_ref(&stroke), &native_contexts);
        assert!(
            native_bindings.is_empty(),
            "{key} must not reach embedded views"
        );
        let (app_bindings, _) = keymap.bindings_for_input(&[stroke], &app_contexts);
        assert!(!app_bindings.is_empty(), "{key} missing in app context");
    }

    let control_map = gpui::Keymap::new(
        registry_for_platform("ctrl")
            .into_iter()
            .map(|shortcut| shortcut.binding)
            .collect(),
    );
    for key in ["ctrl-j", "ctrl-k"] {
        let (bindings, _) = control_map.bindings_for_input(
            &[gpui::Keystroke::parse(key).expect("test operation should succeed")],
            &app_contexts,
        );
        assert!(
            bindings.is_empty(),
            "{key} must stay available to chat input when Control is the modifier"
        );
    }
}

#[test]
fn workgraph_backspace_does_not_navigate_from_inputs() {
    let keymap = gpui::Keymap::new(bindings());
    let stroke = gpui::Keystroke::parse("backspace").expect("test operation should succeed");
    let board = gpui::KeyContext::parse(crate::app::WORKGRAPH_KEY_CONTEXT)
        .expect("test operation should succeed");
    let (matches, _) =
        keymap.bindings_for_input(std::slice::from_ref(&stroke), std::slice::from_ref(&board));
    assert!(!matches.is_empty());
    let (matches, _) = keymap.bindings_for_input(
        &[stroke],
        &[
            board,
            gpui::KeyContext::parse("Input").expect("test operation should succeed"),
        ],
    );
    assert!(matches.is_empty());
}

#[test]
fn copy_shortcuts_route_through_the_application_command() {
    let shortcuts = registry();
    assert!(shortcuts.iter().any(|shortcut| {
        shortcut.label == "Copy transcript selection" && shortcut.keystroke == platform!("c")
    }));
    assert!(shortcuts.iter().any(|shortcut| {
        shortcut.label == "Copy selection" && shortcut.keystroke == platform!("c")
    }));
}
