use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Context, InteractiveElement as _, Keystroke, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use gpui_component::{
    IndexPath,
    kbd::Kbd,
    list::{ListDelegate, ListItem, ListState},
};

use crate::{
    app::RemoveProject,
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, app_icon, icon_control},
    app::ui::theme::THEME,
};

#[derive(Clone)]
pub(crate) struct PickerRow {
    pub(crate) id: String,
    pub(crate) icon: AppIcon,
    pub(crate) label: String,
    pub(crate) detail: Option<String>,
    pub(crate) shortcut: Option<&'static str>,
    removable_project: Option<std::path::PathBuf>,
    search: String,
}

impl PickerRow {
    pub(crate) fn new(
        id: impl Into<String>,
        icon: AppIcon,
        label: impl Into<String>,
        detail: Option<String>,
        shortcut: Option<&'static str>,
        keywords: &str,
    ) -> Self {
        let label = label.into();
        let search = format!(
            "{label} {} {keywords}",
            detail.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        Self {
            id: id.into(),
            icon,
            label,
            detail,
            shortcut,
            removable_project: None,
            search,
        }
    }

    pub(crate) fn removable_project(mut self, project: std::path::PathBuf) -> Self {
        self.removable_project = Some(project);
        self
    }

    fn matches(&self, query: &str) -> bool {
        query
            .split_whitespace()
            .all(|term| self.search.contains(&term.to_lowercase()))
    }
}

pub(crate) struct PickerDelegate {
    all_rows: Vec<PickerRow>,
    visible_rows: Vec<PickerRow>,
    selected_index: Option<IndexPath>,
    confirmed_id: Rc<RefCell<Option<String>>>,
    query: Rc<RefCell<String>>,
}

pub(crate) struct PickerHandles {
    pub(crate) confirmed_id: Rc<RefCell<Option<String>>>,
    pub(crate) query: Rc<RefCell<String>>,
}

impl PickerDelegate {
    pub(crate) fn new(rows: Vec<PickerRow>) -> (Self, PickerHandles) {
        let confirmed_id = Rc::new(RefCell::new(None));
        let query = Rc::new(RefCell::new(String::new()));
        (
            Self {
                visible_rows: rows.clone(),
                all_rows: rows,
                selected_index: Some(IndexPath::default()),
                confirmed_id: Rc::clone(&confirmed_id),
                query: Rc::clone(&query),
            },
            PickerHandles {
                confirmed_id,
                query,
            },
        )
    }
}

impl ListDelegate for PickerDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> gpui::Task<()> {
        *self.query.borrow_mut() = query.to_owned();
        self.visible_rows = self
            .all_rows
            .iter()
            .filter(|row| row.matches(query.trim()))
            .cloned()
            .collect();
        self.selected_index = (!self.visible_rows.is_empty()).then_some(IndexPath::default());
        gpui::Task::ready(())
    }

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.visible_rows.len()
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.visible_rows.get(index.row)?.clone();
        Some(
            ListItem::new(("picker-row", index.row))
                .h(if row.detail.is_some() {
                    THEME.controls.archived_preview_row
                } else {
                    THEME.controls.utility_row
                })
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(THEME.space.sm)
                        .child(app_icon(row.icon, AppIconSize::Control))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(row.label.clone()),
                                )
                                .children(row.detail.map(|detail| {
                                    div()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(detail)
                                })),
                        )
                        .children(row.shortcut.map(|shortcut| {
                            Kbd::new(
                                Keystroke::parse(shortcut)
                                    .expect("static picker shortcut must parse"),
                            )
                            .outline()
                        }))
                        .children(row.removable_project.map(|project| {
                            icon_control(
                                ("remove-picker-project", index.row),
                                format!("Remove {}", row.label),
                            )
                            .hover(|button| button.bg(THEME.colors.hover))
                            .child(app_icon(AppIcon::X, AppIconSize::Control))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(
                                    Box::new(RemoveProject {
                                        path: project.clone(),
                                    }),
                                    cx,
                                );
                            })
                        })),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> impl gpui::IntoElement {
        div()
            .p(THEME.space.md)
            .text_color(THEME.colors.subtle)
            .child("No matches")
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = index;
    }

    fn confirm(&mut self, _: bool, _: &mut Window, _: &mut Context<ListState<Self>>) {
        *self.confirmed_id.borrow_mut() = self
            .selected_index
            .and_then(|index| self.visible_rows.get(index.row))
            .map(|row| row.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_labels_details_and_keywords_by_term() {
        let row = PickerRow::new(
            "session",
            AppIcon::MagnifyingGlass,
            "Find session",
            Some("/work/pi".into()),
            None,
            "resume thread",
        );

        assert!(row.matches("find pi"));
        assert!(row.matches("resume"));
        assert!(!row.matches("project settings"));
    }
}
