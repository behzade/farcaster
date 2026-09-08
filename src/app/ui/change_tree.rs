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
    pub(crate) fn with_default(project: &Path, open: bool) -> Self {
        let mut state = Self::default();
        state.set_all(project, open);
        state
    }

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
#[path = "change_tree_tests.rs"]
mod tests;
