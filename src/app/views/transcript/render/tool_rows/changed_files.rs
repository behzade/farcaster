use std::{collections::BTreeMap, path::Path};

use super::*;

struct ChangedFile {
    path: String,
    label: String,
    operations: Vec<usize>,
    counts: Option<(usize, usize)>,
}

fn collect<'a>(
    items: impl Iterator<Item = (usize, &'a TranscriptItem)>,
    project: Option<&Path>,
    home: Option<&Path>,
) -> Vec<ChangedFile> {
    let mut files = BTreeMap::<String, ChangedFile>::new();
    for (index, item) in items {
        let is_change = item.tool_presentation.is_some()
            || item.tool_details.as_ref().is_some_and(|details| {
                details.metadata.category == Some(crate::agents::ToolCategory::Change)
            });
        if !is_change {
            continue;
        }
        for target in file_targets(item) {
            let path = tool_changes::file_path(target, project);
            let path = path.to_string_lossy().into_owned();
            let counts = item
                .tool_presentation
                .as_ref()
                .filter(|presentation| presentation.path() == target)
                // A multi-file presentation can contain aggregate counts.
                .filter(|_| {
                    item.tool_details
                        .as_ref()
                        .is_none_or(|details| details.metadata.targets.len() <= 1)
                })
                .map(|presentation| presentation.counts())
                .filter(|counts| *counts != (0, 0));
            let file = files.entry(path.clone()).or_insert_with(|| ChangedFile {
                label: tool_changes::file_label(&path, project, home),
                path,
                operations: Vec::new(),
                counts,
            });
            if file.operations.last() != Some(&index) {
                file.operations.push(index);
            }
            // Repeated patches are not a net diff. Keep each operation inspectable.
            if file.operations.len() > 1 {
                file.counts = None;
            }
        }
    }
    files.into_values().collect()
}

pub(super) fn render(
    key: usize,
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    show_details: bool,
    selected_file: Option<&str>,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let project = entity
        .upgrade()
        .map(|entity| entity.read(cx).workspace_project());
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let files = collect(
        items
            .iter_range(start..start + len)
            .enumerate()
            .map(|(offset, item)| (start + offset, item.as_ref())),
        project.as_deref(),
        home.as_deref(),
    );
    let compact = files.len() <= 2;
    let selected = files
        .iter()
        .find(|file| selected_file == Some(file.path.as_str()));
    let mut directories = BTreeMap::<&str, Vec<&ChangedFile>>::new();
    for file in &files {
        let directory = if compact {
            ""
        } else {
            Path::new(&file.label)
                .parent()
                .and_then(Path::to_str)
                .unwrap_or("")
        };
        directories.entry(directory).or_default().push(file);
    }

    div()
        .id(("activity-files", key))
        .w_full()
        .flex()
        .flex_col()
        .children(directories.into_iter().map(|(directory, files)| {
            let nested = !directory.is_empty();
            div()
                .w_full()
                .flex()
                .flex_col()
                .py(THEME.space.xs)
                .when(nested, |section| {
                    section.child(
                        div()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.muted)
                            .child(format!("{directory}/")),
                    )
                })
                .children(files.into_iter().map(|file| {
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .when(nested, |row| row.pl(THEME.space.sm))
                        .child(file_row(
                            key,
                            file,
                            compact,
                            show_details && selected_file == Some(file.path.as_str()),
                            entity.clone(),
                        ))
                }))
        }))
        .when_some(selected.filter(|_| show_details), |section, file| {
            section.child(
                div()
                    .id(("file-history", key))
                    .w_full()
                    .max_h(THEME.layout.tool_max_height)
                    .overflow_y_scroll()
                    .border_l(THEME.border)
                    .border_color(THEME.colors.subtle)
                    .pl(THEME.space.sm)
                    .children(file.operations.iter().map(|&operation| {
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .children(file_links(operation, &items[operation], entity.clone()))
                            .child(expanded_tool_body(
                                ("file-operation", operation),
                                &items[operation],
                            ))
                    })),
            )
        })
        .into_any_element()
}

fn file_row(
    key: usize,
    file: &ChangedFile,
    compact: bool,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let label = if compact {
        file.label.clone()
    } else {
        Path::new(&file.label)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let path = file.path.clone();
    tool_changes::title_row(
        format!("change-file-{key}-{path}"),
        format!(
            "{} changes to {} from this activity",
            if expanded { "Hide" } else { "Show" },
            file.label
        ),
        move |_, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.set_transcript_file_details(key, (!expanded).then(|| path.clone()), cx);
            });
        },
    )
    .aria_expanded(expanded)
    .text_size(THEME.type_scale.body_small)
    .child(
        div()
            .flex_1()
            .min_w_0()
            .font_family(MONO_FONT_FAMILY)
            .text_color(THEME.colors.text)
            .child(label),
    )
    .children(tool_changes::change_counts(file.counts.unwrap_or_default()))
    .when(file.operations.len() > 1, |row| {
        row.child(tool_changes::tool_label(format!(
            "{} edits",
            file.operations.len()
        )))
    })
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_files_keep_history_without_summing_patch_counts() {
        let first = super::super::tests::write_item();
        let mut second = first.clone();
        let details = Arc::make_mut(second.tool_details.as_mut().unwrap());
        details.metadata.targets = vec!["/repo/src/main.rs".into(), "src/other.rs".into()];
        let files = collect(
            [(7, &first), (9, &second)].into_iter(),
            Some(Path::new("/repo")),
            None,
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].label, "src/main.rs");
        assert_eq!(files[0].operations, [7, 9]);
        assert_eq!(files[0].counts, None);
        assert_eq!(files[1].label, "src/other.rs");
        assert_eq!(files[1].counts, None);
    }

    #[test]
    fn native_multi_file_patch_shows_every_target_without_aggregate_counts() {
        use crate::app::views::transcript::conversation::ConversationState;
        use serde_json::json;
        let mut state = ConversationState::default();
        state.reduce(&json!({
            "type":"tool_execution_start", "toolCallId":"patch", "toolName":"edit",
            "args":{"path":"src/one/mod.rs", "changes":[
                {"path":"src/one/mod.rs","diff":"+one\n-two"},
                {"path":"src/two/mod.rs","diff":"+three"}
            ]},
            "toolMetadata":{"category":"change", "targets":["src/one/mod.rs", "src/two/mod.rs", "/outside/config"]}
        }));
        state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"patch", "isError":false, "result":{"content":[]}}));
        let files = collect(
            [(3, state.items[0].as_ref())].into_iter(),
            Some(Path::new("/repo")),
            None,
        );
        assert_eq!(
            files
                .iter()
                .map(|file| file.label.as_str())
                .collect::<Vec<_>>(),
            ["/outside/config", "src/one/mod.rs", "src/two/mod.rs"]
        );
        assert!(files.iter().all(|file| file.counts.is_none()));
    }
}
