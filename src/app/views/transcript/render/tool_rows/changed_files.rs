use std::{collections::BTreeMap, path::Path};

use super::*;
use crate::app::ui::{
    change_tree::{self, ChangeTreeState, TreeRow},
    file_icons::file_icon,
};
use crate::app::views::transcript::net_changes::{Edit, NetChanges, unified_edits};

struct ChangedFile {
    path: String,
    label: String,
    last_operation: Option<usize>,
    counts: Option<(usize, usize)>,
    line: Option<u64>,
    net: Option<NetChanges>,
}

fn recorded_edits(item: &TranscriptItem, path: &str, project: Option<&Path>) -> Option<Vec<Edit>> {
    let details = item.tool_details.as_ref()?;
    if let Some(changes) = details
        .arguments
        .get("changes")
        .and_then(serde_json::Value::as_array)
    {
        let mut edits = Vec::new();
        for change in changes {
            let target = change.get("path").and_then(serde_json::Value::as_str)?;
            if tool_changes::file_path(target, project) == Path::new(path) {
                edits.extend(unified_edits(change.get("diff")?.as_str()?)?);
            }
        }
        return (!edits.is_empty()).then_some(edits);
    }
    if details.metadata.targets.len() > 1 {
        return None;
    }
    let result = details.result.as_ref();
    let diff = result
        .and_then(|result| result.pointer("/details/unifiedDiff"))
        .or_else(|| result.and_then(|result| result.pointer("/details/diff")))
        .or_else(|| details.arguments.get("diff"))?
        .as_str()?;
    unified_edits(diff)
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
                last_operation: None,
                counts,
                line: file_target_line(item, target),
                net: Some(NetChanges::default()),
            });
            if file.last_operation == Some(index) {
                continue;
            }
            if file.last_operation.replace(index).is_some() {
                file.counts = None;
            }
            file.line = file_target_line(item, target);
            let edits = recorded_edits(item, &file.path, project);
            let applied = file
                .net
                .as_mut()
                .zip(edits.as_deref())
                .is_some_and(|(net, edits)| net.apply(edits).is_some());
            if !applied {
                file.net = None;
            }
        }
    }
    files
        .into_values()
        .map(|mut file| {
            if let Some(net) = file.net.take() {
                file.counts = net.counts();
            }
            file
        })
        .collect()
}

pub(super) fn render(
    key: usize,
    items: &PersistentVec<Arc<TranscriptItem>>,
    start: usize,
    len: usize,
    state: Option<&ChangeTreeState>,
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
    let project = project.unwrap_or_default();
    let default_state = ChangeTreeState::default();
    let rows = change_tree::rows(
        files
            .iter()
            .enumerate()
            .map(|(index, file)| (index, Path::new(&file.label), None)),
        "",
        &project,
        state.unwrap_or(&default_state),
    );

    div()
        .id(("activity-files", key))
        .w_full()
        .flex()
        .flex_col()
        .children(rows.into_iter().map(|row| {
            match row {
                TreeRow::Folder {
                    path,
                    label,
                    depth,
                    open,
                    ..
                } => {
                    let entity = entity.clone();
                    let project = project.clone();
                    tool_changes::title_row(
                        format!("activity-folder-{key}-{}", path.display()),
                        format!(
                            "{} folder {}",
                            if open { "Collapse" } else { "Expand" },
                            path.display()
                        ),
                        move |_, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.toggle_transcript_folder(key, &project, &path, cx);
                            });
                        },
                    )
                    .aria_expanded(open)
                    .h(px(22.0))
                    .pl(px(depth as f32 * 12.0))
                    .text_size(THEME.type_scale.body_small)
                    .text_color(THEME.colors.muted)
                    .child(app_icon(
                        if open {
                            AppIcon::CaretDown
                        } else {
                            AppIcon::CaretRight
                        },
                        AppIconSize::Inline,
                    ))
                    .child(div().min_w_0().text_ellipsis().child(label))
                    .into_any_element()
                }
                TreeRow::File { index, depth } => div()
                    .w_full()
                    .pl(px(depth as f32 * 12.0))
                    .child(file_row(key, &files[index], entity.clone()))
                    .into_any_element(),
            }
        }))
        .into_any_element()
}

fn file_row(key: usize, file: &ChangedFile, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let label = Path::new(&file.label)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let path = file.path.clone();
    let line = file.line;
    tool_changes::title_row(
        format!("change-file-{key}-{path}"),
        format!("Edit {}", file.label),
        move |window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.open_file_editor_at_line(path.clone().into(), line, window, cx);
            });
        },
    )
    .h(px(22.0))
    .text_size(THEME.type_scale.body_small)
    .child(file_icon(Path::new(&file.path)))
    .child(
        div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_color(THEME.colors.text)
            .child(label),
    )
    .children(tool_changes::change_counts(file.counts.unwrap_or_default()))
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_file_edits_keep_independent_net_counts() {
        let mut first = super::super::tests::write_item();
        let details = Arc::make_mut(first.tool_details.as_mut().unwrap());
        details.metadata.targets = vec!["src/main.rs".into(), "src/other.rs".into()];
        details.arguments = serde_json::json!({"changes":[
            {"path":"src/main.rs", "diff":"@@ -1 +1 @@\n-old\n+middle"},
            {"path":"src/other.rs", "diff":"@@ -0,0 +1,2 @@\n+one\n+two"}
        ]});
        let mut second = first.clone();
        let details = Arc::make_mut(second.tool_details.as_mut().unwrap());
        details.metadata.targets = vec!["src/main.rs".into()];
        details.arguments = serde_json::json!({"changes":[
            {"path":"/repo/src/main.rs", "diff":"@@ -1 +1 @@\n-middle\n+final"}
        ]});
        let files = collect(
            [(0, &first), (1, &second)].into_iter(),
            Some(Path::new("/repo")),
            None,
        );
        assert_eq!(files[0].counts, Some((1, 1)));
        assert_eq!(files[1].counts, Some((2, 0)));
    }

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
        assert_eq!(files[0].last_operation, Some(9));
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
