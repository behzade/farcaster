use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const LARGE_CHANGESET: usize = 20;

#[derive(Clone, Default)]
pub(crate) struct ChangeTreeState {
    projects: BTreeMap<PathBuf, FolderState>,
}

#[derive(Clone, Default)]
struct FolderState {
    default_open: Option<bool>,
    overrides: BTreeMap<PathBuf, bool>,
}

impl ChangeTreeState {
    pub(crate) fn observe(&mut self, project: &Path, count: usize) {
        if count > 0 {
            self.projects
                .entry(project.into())
                .or_default()
                .default_open
                .get_or_insert(count <= LARGE_CHANGESET);
        }
    }

    pub(crate) fn is_open(&self, project: &Path, folder: &Path) -> bool {
        self.projects.get(project).map_or(true, |state| {
            state
                .overrides
                .get(folder)
                .copied()
                .unwrap_or(state.default_open.unwrap_or(true))
        })
    }

    pub(crate) fn toggle(&mut self, project: &Path, folder: &Path) {
        let open = !self.is_open(project, folder);
        self.projects
            .entry(project.into())
            .or_default()
            .overrides
            .insert(folder.into(), open);
    }

    pub(crate) fn set_all(&mut self, project: &Path, open: bool) {
        self.projects.insert(
            project.into(),
            FolderState {
                default_open: Some(open),
                overrides: BTreeMap::new(),
            },
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TreeRow {
    Folder {
        path: PathBuf,
        label: String,
        count: usize,
        counts: Option<(usize, usize)>,
        depth: usize,
        open: bool,
    },
    File {
        index: usize,
        depth: usize,
    },
}

struct Node {
    folders: BTreeMap<String, Node>,
    files: Vec<(String, usize)>,
    count: usize,
    counts: Option<(usize, usize)>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            folders: BTreeMap::new(),
            files: Vec::new(),
            count: 0,
            counts: Some((0, 0)),
        }
    }
}

pub(crate) fn rows<'a>(
    files: impl Iterator<Item = (usize, &'a Path, Option<&'a Path>, Option<(usize, usize)>)>,
    query: &str,
    project: &Path,
    state: &ChangeTreeState,
) -> Vec<TreeRow> {
    let query = query.trim().to_lowercase();
    let mut root = Node::default();
    let mut seen = BTreeSet::new();
    for (index, path, original, counts) in files {
        if !query.is_empty()
            && !path.to_string_lossy().to_lowercase().contains(&query)
            && !original.is_some_and(|path| path.to_string_lossy().to_lowercase().contains(&query))
        {
            continue;
        }
        let increment = usize::from(seen.insert(path));
        let mut node = &mut root;
        node.count += increment;
        node.counts = sum_counts(node.counts, counts);
        if let Some(parent) = path.parent() {
            for part in parent.components() {
                node = node
                    .folders
                    .entry(part.as_os_str().to_string_lossy().into_owned())
                    .or_default();
                node.count += increment;
                node.counts = sum_counts(node.counts, counts);
            }
        }
        node.files.push((
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            index,
        ));
    }
    let mut result = Vec::new();
    flatten(
        root,
        Path::new(""),
        0,
        !query.is_empty(),
        project,
        state,
        &mut result,
    );
    result
}

fn flatten(
    mut node: Node,
    parent: &Path,
    depth: usize,
    searching: bool,
    project: &Path,
    state: &ChangeTreeState,
    out: &mut Vec<TreeRow>,
) {
    for (mut label, mut child) in node.folders {
        let mut path = parent.join(&label);
        while child.files.is_empty() && child.folders.len() == 1 {
            let (name, next) = child.folders.pop_first().unwrap();
            path.push(&name);
            if !label.ends_with('/') {
                label.push('/');
            }
            label.push_str(&name);
            child = next;
        }
        let open = searching || state.is_open(project, &path);
        out.push(TreeRow::Folder {
            path: path.clone(),
            label,
            count: child.count,
            counts: child.counts,
            depth,
            open,
        });
        if open {
            flatten(child, &path, depth + 1, searching, project, state, out);
        }
    }
    node.files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    out.extend(
        node.files
            .into_iter()
            .map(|(_, index)| TreeRow::File { index, depth }),
    );
}

fn sum_counts(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
    a.zip(b)
        .map(|(a, b)| (a.0.saturating_add(b.0), a.1.saturating_add(b.1)))
}

#[cfg(test)]
mod tests {
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
}
