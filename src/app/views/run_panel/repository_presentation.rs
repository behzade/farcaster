use std::{
    hash::{Hash as _, Hasher as _},
    path::Path,
};

use crate::{
    app::ui::theme::THEME,
    repository::{
        ChangeKind, ChangeLayer, DiffTargetKey, GitIdentity, SnapshotIdentity, WorkingCopyChange,
    },
};

pub(super) fn repository_row_id(key: &DiffTargetKey) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn git_identity(identity: &GitIdentity) -> String {
    match (identity.branch.as_deref(), identity.head_oid.as_deref()) {
        (Some(branch), Some(_)) => branch.to_owned(),
        (Some(branch), None) => format!("{branch} · unborn"),
        (None, Some(oid)) => format!("detached {}", short_id(oid)),
        (None, None) => "unborn HEAD".to_owned(),
    }
}

pub(super) fn repository_sync_metadata(identity: &SnapshotIdentity) -> String {
    match identity {
        SnapshotIdentity::Git(identity) => {
            let metadata = identity
                .upstream
                .clone()
                .or_else(|| identity.nearest_branch.clone())
                .or_else(|| identity.branch.as_ref().map(|_| "No upstream".to_owned()))
                .unwrap_or_else(|| "detached".to_owned());
            with_ahead_behind(metadata, identity.ahead, identity.behind)
        }
        SnapshotIdentity::Jujutsu(identity) => {
            let metadata = bookmark_metadata(if identity.closest_bookmarks.is_empty() {
                &identity.bookmarks
            } else {
                &identity.closest_bookmarks
            });
            with_ahead_behind(metadata, identity.ahead, 0)
        }
    }
}

fn with_ahead_behind(mut metadata: String, ahead: u64, behind: u64) -> String {
    if ahead > 0 {
        metadata.push_str(&format!(" · {} ahead", ahead));
    }
    if behind > 0 {
        metadata.push_str(&format!(" · {} behind", behind));
    }
    metadata
}

fn bookmark_metadata(bookmarks: &[String]) -> String {
    match bookmarks {
        [] => "No bookmark".to_owned(),
        [bookmark] => bookmark.clone(),
        [first, rest @ ..] => format!("{first} +{} bookmarks", rest.len()),
    }
}

pub(super) const fn group_title(layer: ChangeLayer) -> &'static str {
    match layer {
        ChangeLayer::GitIndex => "Staged",
        ChangeLayer::GitWorkingTree => "Working tree",
        ChangeLayer::GitConflict => "Conflicts",
        ChangeLayer::GitUntracked => "Untracked",
        ChangeLayer::JujutsuWorkingCopy => "Current change",
    }
}

pub(super) fn display_change_path(change: &WorkingCopyChange) -> String {
    let target = visible_path(&change.relative_path);
    change
        .original_relative_path
        .as_ref()
        .map_or(target.clone(), |source| {
            format!("{} -> {target}", visible_path(source))
        })
}

pub(super) fn accessible_change_path(change: &WorkingCopyChange) -> String {
    let target = visible_path(&change.relative_path);
    change
        .original_relative_path
        .as_ref()
        .map_or(target.clone(), |source| {
            let source = visible_path(source);
            match change.kind {
                ChangeKind::Copied => format!("copied from {source} to {target}"),
                _ => format!("renamed from {source} to {target}"),
            }
        })
}

fn visible_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub(super) fn middle_truncate(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars || max_chars < 3 {
        return value.to_owned();
    }
    let left = (max_chars - 1) / 2;
    let right = max_chars - 1 - left;
    chars[..left]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - right..].iter())
        .collect()
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

pub(super) fn bounded_message(message: &str) -> String {
    const LIMIT: usize = 320;
    let normalized = message.replace(['\r', '\n'], " ");
    let mut characters = normalized.chars();
    let bounded = characters.by_ref().take(LIMIT).collect::<String>();
    if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

pub(super) const fn change_kind_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Copied => "copied",
        ChangeKind::TypeChanged => "type-changed",
        ChangeKind::Untracked => "untracked",
        ChangeKind::Conflict => "conflicted",
        ChangeKind::Unknown(_) => "changed",
    }
}

pub(super) fn change_status_label(change: &WorkingCopyChange) -> &str {
    if change.layer == ChangeLayer::JujutsuWorkingCopy && change.kind == ChangeKind::Conflict {
        "!"
    } else {
        change.kind.status_label()
    }
}

pub(super) fn change_color(kind: &ChangeKind) -> gpui::Rgba {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => THEME.colors.success,
        ChangeKind::Deleted | ChangeKind::Conflict => THEME.colors.error,
        ChangeKind::Renamed | ChangeKind::Copied | ChangeKind::TypeChanged => THEME.colors.warning,
        ChangeKind::Modified | ChangeKind::Unknown(_) => THEME.colors.accent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::JujutsuIdentity;

    #[test]
    fn unborn_and_detached_git_heads_are_explicit() {
        assert_eq!(
            git_identity(&GitIdentity {
                branch: Some("main".into()),
                ..GitIdentity::default()
            }),
            "main · unborn"
        );
        assert_eq!(
            git_identity(&GitIdentity {
                head_oid: Some("0123456789abcdef".into()),
                ..GitIdentity::default()
            }),
            "detached 01234567"
        );
    }

    #[test]
    fn sync_metadata_uses_nearest_git_branch_and_jj_ancestor_bookmark() {
        assert_eq!(
            repository_sync_metadata(&SnapshotIdentity::Git(GitIdentity {
                nearest_branch: Some("main".into()),
                ahead: 2,
                ..GitIdentity::default()
            })),
            "main · 2 ahead"
        );
        assert_eq!(
            repository_sync_metadata(&SnapshotIdentity::Jujutsu(JujutsuIdentity {
                operation_id: String::new(),
                commit_id: "commit".into(),
                change_id: "change".into(),
                description: String::new(),
                bookmarks: Vec::new(),
                closest_bookmarks: vec!["main".into()],
                ahead: 2,
                conflicted_paths: Vec::new(),
                conflicted: false,
                empty: true,
            })),
            "main · 2 ahead"
        );
    }

    #[test]
    fn unusual_paths_are_reduced_to_one_visible_line() {
        assert_eq!(
            visible_path(Path::new("old\nname\t.rs")),
            "old\\nname\\t.rs"
        );
    }
}
