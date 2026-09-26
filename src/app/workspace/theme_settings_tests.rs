use super::*;
use crate::app::ui::{
    file_icons,
    theme::{ColorKey, SyntaxKey, ThemeToken, builtin::BUILT_IN_THEMES},
};

fn saved_library() -> String {
    let mut library = ThemeLibrary::default();
    let mut definition = BUILT_IN_THEMES[2].clone();
    definition.name = "Ocean".to_owned();
    definition.colors.canvas = parse_hex("#123456").expect("hex");
    definition.set_color(
        ThemeToken::Syntax(SyntaxKey::keyword),
        parse_hex("#654321").expect("hex"),
    );
    definition.set_color(ThemeToken::Icon(0), parse_hex("#abcdef").expect("hex"));
    library.upsert(definition).expect("save theme");
    library.to_css()
}

#[test]
fn themes_fall_back_to_the_bundled_theme() {
    let settings = ThemeSettings::load(None, None);
    assert!(settings.editable());
    assert_eq!(settings.library.selected_name(), BUILT_IN_THEMES[0].name);
    assert!(settings.draft.is_none());
    assert!(settings.error.is_none());
}

#[test]
fn saved_themes_and_selection_are_restored() {
    let settings = ThemeSettings::load(Some(&saved_library()), Some("Ocean"));
    assert!(settings.editable());
    assert_eq!(settings.library.selected_name(), "Ocean");
    assert_eq!(
        settings.library.selected().colors.get(ColorKey::canvas),
        parse_hex("#123456").expect("hex")
    );
    assert_eq!(settings.library.user_themes().len(), 1);
    assert_eq!(settings.draft.as_ref().unwrap().name, "Ocean");
    assert!(settings.error.is_none());
}

#[test]
fn saved_icon_and_syntax_tokens_are_restored() {
    let settings = ThemeSettings::load(Some(&saved_library()), Some("Ocean"));
    let definition = settings.library.selected();
    assert_eq!(
        definition.color(ThemeToken::Syntax(SyntaxKey::keyword)),
        parse_hex("#654321").expect("hex")
    );
    assert_eq!(
        definition.color(ThemeToken::Icon(0)),
        parse_hex("#abcdef").expect("hex")
    );
    assert_eq!(
        definition.color(ThemeToken::Icon(1)),
        file_icons::native_color(1)
    );
}

#[test]
fn unreadable_saved_themes_keep_the_store_untouched() {
    let settings = ThemeSettings::load(Some("{ not css"), None);
    assert!(!settings.editable());
    assert!(settings.error.is_some());
    assert_eq!(settings.library.selected_name(), BUILT_IN_THEMES[0].name);
}

#[test]
fn every_bundled_theme_can_be_selected() {
    let mut settings = ThemeSettings::default();
    for definition in BUILT_IN_THEMES.iter() {
        settings
            .library
            .select(&definition.name)
            .expect("select bundled theme");
        assert_eq!(settings.library.selected(), *definition);
    }
}

#[test]
fn failed_storage_read_disables_theme_writes() {
    let settings = ThemeSettings::load_failed("database unavailable".into());
    assert!(!settings.editable());
    assert!(settings.draft.is_none());
    assert_eq!(settings.error.as_deref(), Some("database unavailable"));
}

#[gpui::test]
fn failed_theme_load_cannot_replace_saved_themes(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_theme_load_cannot_replace_saved_themes"
        ),
        cx,
        |cx, app, _, _| {
            let css = default_palette_library();
            crate::app::persistence::open()
                .unwrap()
                .save_theme_settings(&css, "Ocean")
                .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings.themes = ThemeSettings::load_failed("read failed".into());
                    // Even a direct action bypassing the disabled controls must not save.
                    app.select_theme(&BUILT_IN_THEMES[0].name, window, cx);
                    assert!(app.persist_themes().is_err());
                    app.flush_theme_save();
                });
            });
            let store = crate::app::persistence::open().unwrap();
            assert_eq!(store.load_theme_css().unwrap(), Some(css));
            assert_eq!(store.load_active_theme().unwrap().as_deref(), Some("Ocean"));
        },
    );
}

#[gpui::test]
fn restored_theme_opens_an_editor_and_quit_flushes_pending_changes(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::restored_theme_opens_an_editor_and_quit_flushes_pending_changes"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings.themes =
                        ThemeSettings::load(Some(&default_palette_library()), Some("Ocean"));
                    app.open_settings(window, cx);
                    assert!(app.settings.themes.name.is_some());
                    assert!(!app.settings.themes.tokens.is_empty());
                    app.commit_theme_rename("Ocean renamed".into(), window, cx);
                    assert!(app.settings.themes.dirty);
                    // Cancelling a quit must not consume the pending save.
                    app.activity
                        .run_statuses
                        .insert("session:busy".into(), "Working".into());
                    app.request_application_quit(window, cx);
                    assert!(app.lifecycle.pending_quit.is_some());
                    app.close_quit_confirmation(window, cx);
                    assert!(app.settings.themes.dirty);
                });
            });
            // Use the App context: shutdown destroys the held window.
            cx.cx.update(|cx| cx.shutdown());
            let store = crate::app::persistence::StateStore::open_at(
                &crate::app::persistence::state_path().unwrap(),
            )
            .unwrap();
            assert_eq!(
                store.load_active_theme().unwrap().as_deref(),
                Some("Ocean renamed")
            );
            let css = store.load_theme_css().unwrap().unwrap();
            let library = ThemeLibrary::from_css(&css, Some("Ocean renamed")).unwrap();
            assert_eq!(library.selected_name(), "Ocean renamed");
        },
    );
}

// App tests may install this definition globally; retain the default visuals.
fn default_palette_library() -> String {
    let mut library = ThemeLibrary::default();
    let mut definition = BUILT_IN_THEMES[0].clone();
    definition.name = "Ocean".into();
    library.upsert(definition).unwrap();
    library.to_css()
}

#[gpui::test]
fn failed_theme_save_stays_dirty_until_retry_succeeds(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_theme_save_stays_dirty_until_retry_succeeds"
        ),
        cx,
        |cx, app, _, _| {
            let css = default_palette_library();
            crate::app::persistence::open()
                .unwrap()
                .save_theme_settings(&css, "Ocean")
                .unwrap();
            let connection =
                rusqlite::Connection::open(crate::app::persistence::state_path().unwrap()).unwrap();
            connection
                .execute_batch(
                    "CREATE TRIGGER reject_theme_selection BEFORE UPDATE ON meta
                 WHEN NEW.key='theme_selected' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
                )
                .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings.themes = ThemeSettings::load(Some(&css), Some("Ocean"));
                    app.commit_theme_rename("Ocean renamed".into(), window, cx);
                    app.flush_theme_save();
                    assert!(app.settings.themes.dirty);
                    assert!(app.settings.themes.error.is_some());
                    assert!(app.settings.themes.save.is_none());
                });
            });
            {
                let store = crate::app::persistence::open().unwrap();
                assert_eq!(store.load_theme_css().unwrap(), Some(css));
                assert_eq!(store.load_active_theme().unwrap().as_deref(), Some("Ocean"));
            }
            connection
                .execute_batch("DROP TRIGGER reject_theme_selection;")
                .unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    // No further edit is needed to retry the unsaved state.
                    app.flush_theme_save();
                    assert!(!app.settings.themes.dirty);
                });
            });
            let store = crate::app::persistence::open().unwrap();
            assert_eq!(
                store.load_active_theme().unwrap().as_deref(),
                Some("Ocean renamed")
            );
            let css = store.load_theme_css().unwrap().unwrap();
            assert_eq!(
                ThemeLibrary::from_css(&css, Some("Ocean renamed"))
                    .unwrap()
                    .selected_name(),
                "Ocean renamed"
            );
        },
    );
}
