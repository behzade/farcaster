//! Pure labels, path projection, and color policy for the Working copy panel.

use std::{
    hash::{Hash as _, Hasher as _},
    path::Path,
};

use crate::{
    repository::{
        BackendPreference, ChangeKind, ChangeLayer, DiffTargetKey, GitIdentity, JujutsuIdentity,
        RepositoryKind, WorkingCopyChange,
    },
    theme::THEME,
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

pub(super) fn git_upstream(identity: &GitIdentity) -> Option<String> {
    let upstream = identity.upstream.as_ref()?;
    let mut label = upstream.clone();
    if identity.ahead > 0 || identity.behind > 0 {
        label.push_str(&format!(" · +{} -{}", identity.ahead, identity.behind));
    }
    Some(label)
}

pub(super) fn jj_identity(identity: &JujutsuIdentity) -> String {
    let mut label = short_id(&identity.change_id);
    if let Some(bookmark) = identity.bookmarks.first() {
        label.push_str(" · ");
        label.push_str(bookmark);
    }
    if identity.conflicted {
        label.push_str(" · conflict");
    }
    label
}

pub(super) fn preference_label(
    preference: BackendPreference,
    active: Option<RepositoryKind>,
) -> String {
    match (preference, active) {
        (BackendPreference::Auto, Some(RepositoryKind::Git)) => "Auto · Git".to_owned(),
        (BackendPreference::Auto, Some(RepositoryKind::Jujutsu)) => "Auto · Jujutsu".to_owned(),
        (BackendPreference::Auto, None) => "Auto".to_owned(),
        (BackendPreference::Git, _) => "Git".to_owned(),
        (BackendPreference::Jujutsu, _) => "Jujutsu".to_owned(),
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

    #[test]
    fn backend_label_names_only_the_active_automatic_backend() {
        assert_eq!(
            preference_label(BackendPreference::Auto, Some(RepositoryKind::Jujutsu)),
            "Auto · Jujutsu"
        );
        assert_eq!(
            preference_label(BackendPreference::Git, Some(RepositoryKind::Jujutsu)),
            "Git"
        );
    }

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
    fn unusual_paths_are_reduced_to_one_visible_line() {
        assert_eq!(
            visible_path(Path::new("old\nname\t.rs")),
            "old\\nname\\t.rs"
        );
    }
}
