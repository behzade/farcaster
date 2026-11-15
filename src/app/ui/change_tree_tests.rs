use super::*;

#[test]
fn grouped_paths_keep_exact_parents_and_shallow_rows() {
    let paths = [
        "src/app/views/dialogs/settings/appearance.rs",
        "src/app/views/dialogs/settings.rs",
        "src/app/views/workgraph/adapter/catalog.rs",
        "src/app/views/workgraph/core.rs",
    ];
    let (prefix, rows) = grouped_rows(
        paths
            .iter()
            .enumerate()
            .map(|(i, path)| (i, Path::new(path), None, Some((1, 2)))),
        "",
        Path::new("/repo"),
        &ChangeTreeState::default(),
    );
    assert_eq!(prefix, Path::new("src/app/views"));
    let groups: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            TreeRow::Folder {
                label,
                depth,
                count,
                ..
            } => Some((label.as_str(), *depth, *count)),
            _ => None,
        })
        .collect();
    assert_eq!(
        groups,
        [
            ("dialogs/settings", 0, 1),
            ("dialogs", 0, 1),
            ("workgraph/adapter", 0, 1),
            ("workgraph", 0, 1)
        ]
    );
    assert!(
        rows.iter()
            .filter_map(|row| match row {
                TreeRow::File { depth, .. } => Some(*depth),
                _ => None,
            })
            .all(|depth| depth == 1)
    );
}

#[test]
fn grouped_search_preserves_prefix_disclosure_layers_and_unknown_counts() {
    let project = Path::new("/repo");
    let mut state = ChangeTreeState::with_default(project, false);
    state.toggle(project, Path::new("src/a"));
    let render = |query| {
        grouped_rows(
            [
                (
                    0,
                    Path::new("src/a/file.rs"),
                    Some(Path::new("old/name.rs")),
                    Some((3, 2)),
                ),
                (1, Path::new("src/a/file.rs"), None, None),
                (2, Path::new("src/b/other.rs"), None, Some((1, 0))),
            ]
            .into_iter(),
            query,
            project,
            &state,
        )
    };
    let (prefix, all) = render("");
    assert_eq!(prefix, Path::new("src"));
    assert!(matches!(
        &all[0],
        TreeRow::Folder {
            count: 1,
            counts: None,
            open: true,
            ..
        }
    ));
    assert_eq!(
        &all[1..3],
        &[
            TreeRow::File { index: 0, depth: 1 },
            TreeRow::File { index: 1, depth: 1 }
        ]
    );
    let (filtered_prefix, filtered) = render("old/name");
    assert_eq!(prefix, filtered_prefix);
    assert!(
        matches!(&filtered[0], TreeRow::Folder { label, counts: Some((3, 2)), open: true, .. } if label == "a")
    );
    assert!(matches!(
        &render("other").1[0],
        TreeRow::Folder { open: true, .. }
    ));
    assert!(!state.is_open(project, Path::new("src/b")));
}

#[test]
fn grouped_paths_keep_root_and_external_paths_distinct() {
    for paths in [
        vec!["src/a.rs"],
        vec!["README", "src/a.rs", "/outside/a.rs", "~/a.rs"],
    ] {
        let (prefix, rows) = grouped_rows(
            paths
                .iter()
                .enumerate()
                .map(|(i, path)| (i, Path::new(path), None, None)),
            "",
            Path::new("/repo"),
            &ChangeTreeState::default(),
        );
        assert!(prefix.as_os_str().is_empty());
        let groups: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                TreeRow::Folder { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        if paths.len() == 1 {
            assert_eq!(groups, ["src"]);
        } else {
            assert_eq!(groups, ["/outside", "src", "~", "Project root"]);
        }
    }
}

#[cfg(unix)]
#[test]
fn lossy_folder_collisions_keep_file_order_and_counts() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let paths = [
        (
            OsString::from_vec(b"src/\xff/\xff.rs".to_vec()).into(),
            Some((1, 2)),
        ),
        (
            OsString::from_vec(b"src/\xfe/a.rs".to_vec()).into(),
            Some((3, 4)),
        ),
        (
            OsString::from_vec(b"src/\xff/a.rs".to_vec()).into(),
            Some((5, 6)),
        ),
        (PathBuf::from("src/é/u.rs"), None),
        (PathBuf::from("src/�/a.rs"), Some((7, 8))),
    ];
    let project = Path::new("/repo");
    let render = |state: &ChangeTreeState| {
        rows(
            paths
                .iter()
                .enumerate()
                .map(|(index, (path, counts))| (index, path.as_path(), None, *counts)),
            "",
            project,
            state,
        )
    };
    assert_eq!(
        render(&ChangeTreeState::with_default(project, false)),
        vec![TreeRow::Folder {
            path: "src".into(),
            label: "src".into(),
            count: 5,
            counts: None,
            depth: 0,
            open: false,
        }],
    );
    assert_eq!(
        render(&ChangeTreeState::with_default(project, true)),
        vec![
            TreeRow::Folder {
                path: "src".into(),
                label: "src".into(),
                count: 5,
                counts: None,
                depth: 0,
                open: true,
            },
            TreeRow::Folder {
                path: "src/é".into(),
                label: "é".into(),
                count: 1,
                counts: None,
                depth: 1,
                open: true,
            },
            TreeRow::File { index: 3, depth: 2 },
            TreeRow::Folder {
                path: "src/�".into(),
                label: "�".into(),
                count: 4,
                counts: Some((16, 20)),
                depth: 1,
                open: true,
            },
            TreeRow::File { index: 1, depth: 2 },
            TreeRow::File { index: 2, depth: 2 },
            TreeRow::File { index: 4, depth: 2 },
            TreeRow::File { index: 0, depth: 2 },
        ],
    );
}

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
