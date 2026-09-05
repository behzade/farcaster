use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const LARGE_CHANGESET: usize = 20;

#[derive(Default)]
pub(crate) struct ChangeTreeState {
    projects: BTreeMap<PathBuf, FolderState>,
}

#[derive(Default)]
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
pub(super) enum TreeRow {
    Folder {
        path: PathBuf,
        label: String,
        count: usize,
        depth: usize,
        open: bool,
    },
    File {
        index: usize,
        depth: usize,
    },
}

#[derive(Default)]
struct Node {
    folders: BTreeMap<String, Node>,
    files: Vec<(String, usize)>,
    count: usize,
}

pub(super) fn rows<'a>(
    files: impl Iterator<Item = (usize, &'a Path, Option<&'a Path>)>,
    query: &str,
    project: &Path,
    state: &ChangeTreeState,
) -> Vec<TreeRow> {
    let query = query.trim().to_lowercase();
    let mut root = Node::default();
    let mut seen = BTreeSet::new();
    for (index, path, original) in files {
        if !query.is_empty()
            && !path.to_string_lossy().to_lowercase().contains(&query)
            && !original.is_some_and(|path| path.to_string_lossy().to_lowercase().contains(&query))
        {
            continue;
        }
        let increment = usize::from(seen.insert(path));
        let mut node = &mut root;
        node.count += increment;
        if let Some(parent) = path.parent() {
            for part in parent.components() {
                node = node
                    .folders
                    .entry(part.as_os_str().to_string_lossy().into_owned())
                    .or_default();
                node.count += increment;
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
            label.push('/');
            label.push_str(&name);
            child = next;
        }
        let open = searching || state.is_open(project, &path);
        out.push(TreeRow::Folder {
            path: path.clone(),
            label,
            count: child.count,
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

#[cfg(test)]
mod tests {
    use super::*;

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
                paths.iter().enumerate().map(|(i, path)| (i, *path, None)),
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
        // A staged and an unstaged row for the same file count as one file.
        paths.push(paths[0].clone());
        let mut state = ChangeTreeState::default();
        state.observe(project, paths.len());
        let render = |state: &ChangeTreeState| {
            rows(
                paths
                    .iter()
                    .enumerate()
                    .map(|(i, path)| (i, path.as_path(), None)),
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
            std::iter::once((0, Path::new("new/file.rs"), Some(Path::new("old/name.rs")))),
            "old/name",
            Path::new("/project"),
            &state,
        );
        assert!(result.contains(&TreeRow::File { index: 0, depth: 1 }));
    }
}
