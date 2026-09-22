use super::*;
use crate::app::ui::assets::AppIcon;
use tempfile::tempdir;

#[test]
fn every_editor_opens_files_and_only_neovim_opens_remote_views() {
    let file = PathBuf::from("/tmp/review.rs");
    assert!(matches!(
        editor_target_for_editor(true, EditorTarget::Diff(file.clone(), Some(7))),
        Some(EditorTarget::Diff(path, Some(7))) if path == file
    ));
    assert!(matches!(
        editor_target_for_editor(true, EditorTarget::Transcript("notes".into())),
        Some(EditorTarget::Transcript(_))
    ));
    assert!(matches!(
        editor_target_for_editor(false, EditorTarget::File(file.clone(), Some(3))),
        Some(EditorTarget::File(path, Some(3))) if path == file
    ));
    assert!(matches!(
        editor_target_for_editor(false, EditorTarget::Resume),
        Some(EditorTarget::Resume)
    ));
    assert!(matches!(
        editor_target_for_editor(false, EditorTarget::Diff(file.clone(), Some(9))),
        Some(EditorTarget::File(path, Some(9))) if path == file
    ));
    assert!(editor_target_for_editor(false, EditorTarget::Transcript("notes".into())).is_none());
    assert!(
        editor_target_for_editor(
            false,
            EditorTarget::ReviewLocation {
                list_id: 1,
                index: 0,
                path: file,
            }
        )
        .is_none()
    );
}

#[test]
fn only_file_targets_start_an_editor_on_a_file() {
    let file = PathBuf::from("/tmp/review.rs");
    let diff = editor_target_for_editor(false, EditorTarget::Diff(file.clone(), Some(9)))
        .expect("a diff is a file for every editor");
    assert_eq!(diff.file(), Some(EditorFile::new(file.clone(), Some(9))));
    assert_eq!(
        editor_target_for_editor(false, EditorTarget::File(file, None))
            .expect("files open")
            .file(),
        Some(EditorFile::new(PathBuf::from("/tmp/review.rs"), None))
    );
    assert_eq!(EditorTarget::Resume.file(), None);
    assert_eq!(EditorTarget::Transcript("notes".into()).file(), None);
}

#[test]
fn editor_completion_is_scoped_to_its_request_session_and_view() {
    assert!(editor_completion_is_current(
        1,
        1,
        11,
        Some(11),
        AppSurface::Editor
    ));
    for (generation, tab, surface) in [
        (2, Some(11), AppSurface::Editor),
        (1, Some(22), AppSurface::Editor),
        (1, None, AppSurface::Editor),
        (1, Some(11), AppSurface::Chat),
        (1, Some(11), AppSurface::Terminal),
        (1, Some(11), AppSurface::Work),
    ] {
        assert!(!editor_completion_is_current(
            1, generation, 11, tab, surface
        ));
    }
}

#[gpui::test]
fn editor_choice_persists_normalizes_and_drives_the_surface(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::editor_choice_persists_normalizes_and_drives_the_surface"
        ),
        cx,
        |cx, app, _, _| {
            let stored = || {
                crate::app::persistence::open()
                    .and_then(|store| store.load_text_editor())
                    .expect("load text editor setting")
            };
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(app.settings.text_editor, None);
                    assert_eq!(stored(), None);

                    app.set_settings_text_editor(Some("  mymicro   -p ".into()), window, cx);
                    assert_eq!(app.settings.text_editor.as_deref(), Some("mymicro -p"));
                    assert_eq!(stored().as_deref(), Some("mymicro -p"));
                    assert_eq!(app.text_editor_name(), "mymicro");
                    assert_eq!(app.text_editor_icon(), AppIcon::Code);
                    let command = app.text_editor_command().expect("command resolves");
                    assert_eq!(command.arguments, vec!["-p".to_owned()]);
                    assert!(!command.is_neovim());
                    assert_eq!(
                        app.settings.text_editor_input.read(cx).value(),
                        "mymicro -p"
                    );
                    assert_eq!(app.settings.text_editor_error, None);
                    app.open_settings(window, cx);
                });
                window.draw(cx).clear(cx);
                app.update(cx, |app, cx| {
                    app.set_settings_text_editor(Some("/opt/homebrew/bin/nvim".into()), window, cx);
                    assert_eq!(app.text_editor_name(), "Neovim");
                    assert_eq!(app.text_editor_icon(), AppIcon::Neovim);
                    assert!(
                        app.text_editor_command()
                            .expect("command resolves")
                            .is_neovim()
                    );

                    app.set_settings_text_editor(Some("nvim '/tmp/open".into()), window, cx);
                    assert!(app.settings.text_editor_error.is_some());
                    assert_eq!(
                        app.settings.text_editor.as_deref(),
                        Some("/opt/homebrew/bin/nvim")
                    );
                    assert_eq!(stored().as_deref(), Some("/opt/homebrew/bin/nvim"));

                    app.set_settings_text_editor(None, window, cx);
                    assert_eq!(app.settings.text_editor, None);
                    assert_eq!(stored(), None);
                    assert_eq!(app.settings.text_editor_input.read(cx).value(), "");
                    assert_eq!(app.settings.text_editor_error, None);
                    assert_ne!(app.text_editor_name(), "mymicro");
                    if let Ok(command) = app.text_editor_command() {
                        assert_ne!(command.program, "mymicro");
                    }
                });
                window.draw(cx).clear(cx);
            });
        },
    );
}

#[test]
fn editor_paths_allow_targets_outside_the_selected_project()
-> Result<(), Box<dyn std::error::Error>> {
    let project = tempdir()?;
    let file = project.path().join("src.rs");
    std::fs::write(&file, "fn main() {}")?;
    assert_eq!(
        resolve_editor_path(project.path(), Path::new("src.rs"))?,
        file.canonicalize()?
    );
    assert_eq!(
        resolve_editor_path(project.path(), Path::new("deleted.rs"))?,
        project.path().canonicalize()?.join("deleted.rs")
    );
    let outside = tempdir()?;
    let outside_file = outside.path().join("outside.rs");
    std::fs::write(&outside_file, "")?;
    assert_eq!(
        resolve_editor_path(project.path(), &outside_file)?,
        outside_file.canonicalize()?
    );
    let new_outside_file = outside.path().join("new.rs");
    assert_eq!(
        resolve_editor_path(project.path(), &new_outside_file)?,
        outside.path().canonicalize()?.join("new.rs")
    );
    #[cfg(unix)]
    {
        let dangling = project.path().join("dangling.rs");
        std::os::unix::fs::symlink(outside.path().join("missing.rs"), &dangling)?;
        assert!(resolve_editor_path(project.path(), &dangling).is_err());
    }
    Ok(())
}
