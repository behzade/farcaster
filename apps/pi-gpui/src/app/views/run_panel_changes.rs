//! Compact session-wide changed-file list.

use std::path::Path;

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    session_changes::{FileChange, FileChangeKind},
    sessions::root_session_for_path,
    theme::{MONO_FONT_FAMILY, THEME},
};

const MAX_VISIBLE_CHANGE_FILES: usize = 5;

impl PiApp {
    pub(super) fn render_changes(&self, entity: WeakEntity<Self>) -> AnyElement {
        if self.changes.set.files.is_empty() {
            let (message, color) = if self.changes.set.incomplete {
                (
                    "The session record was too large to scan in full; some changes may be missing",
                    THEME.colors.warning,
                )
            } else {
                (
                    "No successful edit or write calls were recorded",
                    THEME.colors.subtle,
                )
            };
            return div()
                .text_size(THEME.type_scale.caption)
                .text_color(color)
                .child(message)
                .into_any_element();
        }
        let additions = sum_known_counts(self.changes.set.files.iter().map(|file| file.additions))
            .map_or_else(|| "+—".into(), |count| format!("+{count}"));
        let deletions = sum_known_counts(self.changes.set.files.iter().map(|file| file.deletions))
            .map_or_else(|| "-—".into(), |count| format!("-{count}"));
        let project = root_session_for_path(
            &self.all_sessions,
            self.snapshot.selected_session.as_deref(),
        )
        .map(|root| root.project.as_path());
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .when(self.changes.set.incomplete, |changes| {
                changes.child(
                    div()
                        .pb(THEME.space.xs)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.warning)
                        .child("The session record was too large to scan in full; some changes may be missing"),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .pb(THEME.space.xs)
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .child(
                        div()
                            .text_color(THEME.colors.subtle)
                            .child(format!("{} files", self.changes.set.files.len())),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(THEME.space.sm)
                            .child(
                                div()
                                    .text_color(THEME.colors.success)
                                    .child(additions),
                            )
                            .child(
                                div()
                                    .text_color(THEME.colors.error)
                                    .child(deletions),
                            ),
                    ),
            )
            .children(
                self.changes
                    .set
                    .files
                    .iter()
                    .take(MAX_VISIBLE_CHANGE_FILES)
                    .filter_map(|file| self.change_row(file, project, entity.clone())),
            )
            .when(
                self.changes.set.files.len() > MAX_VISIBLE_CHANGE_FILES,
                |changes| {
                    changes.child(
                        div()
                            .px(THEME.space.xs)
                            .pt(THEME.space.xs)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(format!(
                                "{} more files",
                                self.changes.set.files.len() - MAX_VISIBLE_CHANGE_FILES
                            )),
                    )
                },
            )
            .into_any_element()
    }

    fn change_row(
        &self,
        file: &FileChange,
        project: Option<&Path>,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let focus = self.changes.row_focus.get(&file.path)?.clone();
        let click_focus = focus.clone();
        let click_file = file.clone();
        let editor_file = file.path.clone();
        let editor_entity = entity.clone();
        let path = file.path.to_string_lossy().into_owned();
        let display_path = middle_truncate(&display_change_path(&file.path, project), 44);
        let state = match file.kind {
            FileChangeKind::Edited => "E",
            FileChangeKind::Written => "W",
            FileChangeKind::Mixed => "M",
        };
        let additions = file
            .additions
            .map_or_else(|| "—".into(), |count| format!("+{count}"));
        let deletions = file
            .deletions
            .map_or_else(|| "—".into(), |count| format!("-{count}"));
        Some(
            div()
                .id(format!("change-row-{path}"))
                .track_focus(&focus)
                .role(Role::Button)
                .aria_label(format!("Open recorded changes for {path}"))
                .tab_index(0)
                .px(THEME.space.xs)
                .py(px(6.0))
                .rounded(THEME.radius)
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.bg(THEME.colors.selection))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.open_file_diff(click_file.clone(), click_focus.clone(), window, cx)
                    });
                })
                .child(
                    div()
                        .w(px(14.0))
                        .font_family(MONO_FONT_FAMILY)
                        .text_color(match file.kind {
                            FileChangeKind::Edited => THEME.colors.accent,
                            FileChangeKind::Written => THEME.colors.success,
                            FileChangeKind::Mixed => THEME.colors.warning,
                        })
                        .child(state),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(THEME.type_scale.caption)
                        .child(display_path),
                )
                .when(file.diff.partial, |row| {
                    row.child(
                        div()
                            .flex_none()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.warning)
                            .child("partial"),
                    )
                })
                .child(button(
                    format!("edit-change-{path}"),
                    "Edit",
                    ButtonTone::Quiet,
                    true,
                    move |window, cx| {
                        cx.stop_propagation();
                        let _ = editor_entity.update(cx, |this, cx| {
                            this.open_file_editor(editor_file.clone(), window, cx);
                        });
                    },
                ))
                .child(
                    div()
                        .min_w(px(36.0))
                        .flex_none()
                        .text_align(gpui::TextAlign::Right)
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.success)
                        .child(additions),
                )
                .child(
                    div()
                        .min_w(px(36.0))
                        .flex_none()
                        .text_align(gpui::TextAlign::Right)
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.error)
                        .child(deletions),
                )
                .into_any_element(),
        )
    }
}

fn sum_known_counts(counts: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    counts.into_iter().try_fold(0_u64, |total, count| {
        count.map(|count| total.saturating_add(count))
    })
}

fn display_change_path(path: &Path, project: Option<&Path>) -> String {
    project
        .and_then(|project| path.strip_prefix(project).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn middle_truncate(value: &str, max_chars: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_counts_remain_unknown_when_any_file_is_unknown() {
        assert_eq!(sum_known_counts([Some(2), Some(3)]), Some(5));
        assert_eq!(sum_known_counts([Some(2), None]), None);
    }

    #[test]
    fn changed_file_paths_are_relative_to_the_selected_project() {
        assert_eq!(
            display_change_path(
                Path::new("/project/apps/pi-gpui/src/app.rs"),
                Some(Path::new("/project")),
            ),
            "apps/pi-gpui/src/app.rs"
        );
        assert_eq!(
            display_change_path(Path::new("/outside/file.rs"), Some(Path::new("/project"))),
            "/outside/file.rs"
        );
    }
}
