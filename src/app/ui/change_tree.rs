use std::{
    borrow::Cow,
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
        self.projects.get(project).is_none_or(|state| {
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

struct Node<'a> {
    folders: BTreeMap<Cow<'a, str>, Node<'a>>,
    files: Vec<(Cow<'a, str>, usize)>,
    count: usize,
    counts: Option<(usize, usize)>,
}

pub(crate) fn grouped_rows<'a>(
    files: impl Iterator<Item = (usize, &'a Path, Option<&'a Path>, Option<(usize, usize)>)>,
    query: &str,
    project: &Path,
    state: &ChangeTreeState,
) -> (PathBuf, Vec<TreeRow>) {
    let files: Vec<_> = files.collect();
    let mut prefix = files
        .first()
        .and_then(|(_, path, _, _)| path.parent())
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    for (_, path, _, _) in &files {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        while !parent.starts_with(&prefix) {
            if !prefix.pop() {
                prefix.clear();
                break;
            }
        }
    }
    if files
        .iter()
        .any(|(_, path, _, _)| path.parent() == Some(prefix.as_path()))
        && !prefix.pop()
    {
        prefix.clear();
    }
    let query = query.trim().to_lowercase();
    let mut groups = BTreeMap::<PathBuf, Node<'_>>::new();
    let mut seen = BTreeSet::new();
    for (index, path, original, counts) in files {
        if !query.is_empty()
            && !path.to_string_lossy().to_lowercase().contains(&query)
            && !original.is_some_and(|path| path.to_string_lossy().to_lowercase().contains(&query))
        {
            continue;
        }
        let group = groups
            .entry(path.parent().unwrap_or_else(|| Path::new("")).into())
            .or_default();
        group.count += usize::from(seen.insert(path));
        group.counts = sum_counts(group.counts, counts);
        group.files.push((
            path.file_name().unwrap_or_default().to_string_lossy(),
            index,
        ));
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|(a, _), (b, _)| {
        if a != b && a.starts_with(b) {
            std::cmp::Ordering::Less
        } else if a != b && b.starts_with(a) {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    });
    let mut rows = Vec::new();
    for (path, mut group) in groups {
        let label = if path.as_os_str().is_empty() {
            "Project root".into()
        } else {
            path.strip_prefix(&prefix)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned()
        };
        let open = !query.is_empty() || state.is_open(project, &path);
        rows.push(TreeRow::Folder {
            path,
            label,
            count: group.count,
            counts: group.counts,
            depth: 0,
            open,
        });
        if open {
            group
                .files
                .sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            rows.extend(
                group
                    .files
                    .into_iter()
                    .map(|(_, index)| TreeRow::File { index, depth: 1 }),
            );
        }
    }
    (prefix, rows)
}

impl Default for Node<'_> {
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
                    .entry(part.as_os_str().to_string_lossy())
                    .or_default();
                node.count += increment;
                node.counts = sum_counts(node.counts, counts);
            }
        }
        node.files.push((
            path.file_name().unwrap_or_default().to_string_lossy(),
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
    mut node: Node<'_>,
    parent: &Path,
    depth: usize,
    searching: bool,
    project: &Path,
    state: &ChangeTreeState,
    out: &mut Vec<TreeRow>,
) {
    for (label, mut child) in node.folders {
        let mut label = label.into_owned();
        let mut path = parent.join(&label);
        while child.files.is_empty() && child.folders.len() == 1 {
            let (name, next) = child
                .folders
                .pop_first()
                .expect("single folder checked above");
            path.push(name.as_ref());
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
