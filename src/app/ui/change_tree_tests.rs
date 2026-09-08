use super::*;

#[test]
fn activity_tree_shares_folders_and_preserves_external_paths() {
    let paths = [
        Path::new("src/modules/agents/mod.rs"),
        Path::new("src/modules/agents/adapter/mod.rs"),
        Path::new("src/modules/sessions/mod.rs"),
        Path::new("/outside/config"),
        Path::new("~/notes.txt"),
    ];
    let project = Path::new("/repo");
    let mut state = ChangeTreeState::default();
    let render = |state: &ChangeTreeState| {
        rows(
            paths
                .iter()
                .enumerate()
                .map(|(i, path)| (i, *path, None, Some((1, 2)))),
            "",
            project,
            state,
        )
    };
    let expanded = render(&state);
    assert!(expanded.contains(&TreeRow::Folder {
        path: "/outside".into(),
        label: "/outside".into(),
        count: 1,
        counts: Some((1, 2)),
        depth: 0,
        open: true,
    }));
    assert!(expanded.contains(&TreeRow::Folder {
        path: "src/modules".into(),
        label: "src/modules".into(),
        count: 3,
        counts: Some((3, 6)),
        depth: 0,
        open: true,
    }));
    assert!(expanded.contains(&TreeRow::File { index: 0, depth: 2 }));
    assert!(expanded.contains(&TreeRow::File { index: 1, depth: 3 }));
    assert_eq!(
        expanded
            .iter()
            .filter(|row| matches!(row, TreeRow::File { .. }))
            .count(),
        5
    );
    state.toggle(project, Path::new("src/modules/agents"));
    let collapsed = render(&state);
    assert!(
        !collapsed
            .iter()
            .any(|row| matches!(row, TreeRow::File { index: 0 | 1, .. }))
    );
    assert!(collapsed.contains(&TreeRow::File { index: 2, depth: 2 }));
}

#[test]
fn large_changesets_collapse_and_refresh_preserves_choices() {
    let project = Path::new("/project");
    let mut state = ChangeTreeState::default();
    state.observe(project, 50);
    assert!(!state.is_open(project, Path::new("src/app")));
    state.observe(Path::new("/small"), 4);
    assert!(state.is_open(Path::new("/small"), Path::new("src/app")));
    state.toggle(project, Path::new("src/app"));
    state.observe(project, 3);
    assert!(state.is_open(project, Path::new("src/app")));
    assert!(!state.is_open(project, Path::new("tests")));
    state.set_all(project, true);
    assert!(state.is_open(project, Path::new("new/folder")));
    state.set_all(project, false);
    assert!(!state.is_open(project, Path::new("src/app")));
}

#[test]
fn explicit_defaults_preserve_toggles_across_refreshes() {
    let project = Path::new("/repo");
    let folder = Path::new("src");
    for open in [false, true] {
        let mut state = ChangeTreeState::with_default(project, open);
        assert_eq!(state.is_open(project, folder), open);
        state.toggle(project, folder);
        state.observe(project, 50);
        assert_eq!(state.is_open(project, folder), !open);
        assert_eq!(state.is_open(project, Path::new("tests")), open);
    }
}

#[test]
fn search_reveals_full_paths_without_changing_disclosure() {
    let project = Path::new("/project");
    let mut state = ChangeTreeState::default();
    state.observe(project, 50);
    let paths = [
        Path::new("src/app/main.rs"),
        Path::new("src/app/mod.rs"),
        Path::new("Cargo.toml"),
    ];
    let render = |query| {
        rows(
            paths
                .iter()
                .enumerate()
                .map(|(i, path)| (i, *path, None, Some((1, 2)))),
            query,
            project,
            &state,
        )
    };
    assert_eq!(render("").len(), 2);
    assert_eq!(
        render("MAIN"),
        vec![
            TreeRow::Folder {
                path: "src/app".into(),
                label: "src/app".into(),
                count: 1,
                counts: Some((1, 2)),
                depth: 0,
                open: true
            },
            TreeRow::File { index: 0, depth: 1 },
        ]
    );
    assert_eq!(render("").len(), 2);
    assert!(render("missing").is_empty());
}

#[test]
fn fifty_files_are_all_available_and_folder_counts_are_unique() {
    let project = Path::new("/project");
    let mut paths = (0..50)
        .map(|i| PathBuf::from(format!("src/app/file-{i:02}.rs")))
        .collect::<Vec<_>>();
    paths.push(paths[0].clone());
    let mut state = ChangeTreeState::default();
    state.observe(project, paths.len());
    let render = |state: &ChangeTreeState| {
        rows(
            paths
                .iter()
                .enumerate()
                .map(|(i, path)| (i, path.as_path(), None, Some((1, 2)))),
            "",
            project,
            state,
        )
    };
    assert_eq!(
        render(&state),
        vec![TreeRow::Folder {
            path: "src/app".into(),
            label: "src/app".into(),
            count: 50,
            counts: Some((51, 102)),
            depth: 0,
            open: false,
        }]
    );
    state.set_all(project, true);
    let expanded = render(&state);
    assert_eq!(
        expanded
            .iter()
            .filter(|row| matches!(row, TreeRow::File { .. }))
            .count(),
        51
    );
    assert!(expanded.contains(&TreeRow::File {
        index: 49,
        depth: 1
    }));
}

#[test]
fn renamed_files_can_be_found_by_their_original_path() {
    let state = ChangeTreeState::default();
    let result = rows(
        std::iter::once((
            0,
            Path::new("new/file.rs"),
            Some(Path::new("old/name.rs")),
            None,
        )),
        "old/name",
        Path::new("/project"),
        &state,
    );
    assert!(result.contains(&TreeRow::File { index: 0, depth: 1 }));
}

#[test]
fn collapsed_parent_includes_nested_files_and_preserves_unknown_counts() {
    let project = Path::new("/repo");
    let mut state = ChangeTreeState::default();
    state.set_all(project, false);
    let render = |unknown| {
        rows(
            [
                (0, Path::new("src/main.rs"), None, Some((2, 3))),
                (1, Path::new("src/nested/lib.rs"), None, unknown),
            ]
            .into_iter(),
            "",
            project,
            &state,
        )
    };
    for (input, expected) in [(Some((4, 5)), Some((6, 8))), (None, None)] {
        assert!(matches!(render(input).as_slice(), [TreeRow::Folder {
                count: 2, counts, open: false, ..
            }] if *counts == expected));
    }
}
