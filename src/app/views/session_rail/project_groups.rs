use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Rgba, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};

use super::{
    FarcasterApp,
    folders::{FolderRow, folder_rows},
    groups::ActiveSessionItem,
    rendering::session_section_header,
    rows::project_label,
};
use crate::app::session_folders::SessionFolders;
use crate::app::ui::{
    assets::AppIcon,
    primitives::{AppIconSize, app_icon},
    theme::theme,
};

pub(super) fn grouped_rows(
    items: Vec<ActiveSessionItem>,
    folders: &SessionFolders,
    collapsed: &BTreeSet<PathBuf>,
) -> Vec<FolderRow> {
    let mut drafts = Vec::new();
    let mut filed = Vec::new();
    let mut unfiled = Vec::new();
    for item in items {
        if matches!(item, ActiveSessionItem::Draft(_)) {
            drafts.push(item);
        } else if folders.folder_for(item.app_session_id()).is_some() {
            filed.push(item);
        } else {
            unfiled.push(item);
        }
    }
    let mut rows = drafts
        .into_iter()
        .map(|item| FolderRow::Session(Box::new(item)))
        .collect::<Vec<_>>();
    let mut sections = folder_rows(filed, folders);
    sections.pop();
    rows.extend(sections);
    rows.extend(project_rows(unfiled, collapsed));
    rows.push(FolderRow::New);
    rows
}

pub(super) fn project_rows(
    items: Vec<ActiveSessionItem>,
    collapsed: &BTreeSet<PathBuf>,
) -> Vec<FolderRow> {
    let mut unfiled = Vec::new();
    let mut grouped = BTreeMap::<PathBuf, Vec<ActiveSessionItem>>::new();
    for item in items {
        match project_of(&item) {
            Some(project) => grouped.entry(project.to_path_buf()).or_default().push(item),
            None => unfiled.push(item),
        }
    }
    let mut rows = unfiled
        .into_iter()
        .map(|item| FolderRow::Session(Box::new(item)))
        .collect::<Vec<_>>();
    for (project, items) in grouped {
        let collapsed = collapsed.contains(&project);
        rows.push(FolderRow::Project(project, collapsed));
        if !collapsed {
            rows.extend(
                items
                    .into_iter()
                    .map(|item| FolderRow::Session(Box::new(item))),
            );
        }
    }
    rows
}

fn project_of(item: &ActiveSessionItem) -> Option<&Path> {
    match item {
        ActiveSessionItem::Draft(_) => None,
        ActiveSessionItem::Session(item) => Some(&item.session.project),
    }
}

pub(super) fn project_color(project: &Path) -> Rgba {
    let colors = theme().colors;
    let palette = [
        colors.accent,
        colors.code,
        colors.skill,
        colors.file,
        colors.warning,
        colors.error,
        colors.success,
        colors.link,
    ];
    palette[palette_index(project) % palette.len()]
}

fn palette_index(project: &Path) -> usize {
    let mut hash = project
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 29;
    hash as usize
}

pub(super) fn project_header(
    project: PathBuf,
    collapsed: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let label = project_label(&project);
    let color = project_color(&project);
    let toggle_project = project.clone();
    let id = SharedString::from(format!("session-project:{}", project.display()));
    session_section_header()
        .id(id)
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .on_click(move |_, _, cx| {
            let project = toggle_project.clone();
            let _ = entity.update(cx, |this, cx| this.toggle_project_group(&project, cx));
        })
        .child(div().size(px(6.0)).rounded_full().flex_none().bg(color))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(app_icon(
            if collapsed {
                AppIcon::CaretRight
            } else {
                AppIcon::CaretDown
            },
            AppIconSize::Inline,
        ))
        .into_any_element()
}

#[cfg(test)]
#[path = "project_groups_tests.rs"]
mod tests;
