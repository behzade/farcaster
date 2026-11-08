pub(in crate::app) mod agents;
mod background_jobs;
pub(crate) use crate::app::ui::change_tree;
#[cfg(test)]
mod expanded_tests;
#[cfg(test)]
mod order_tests;
mod performance;
mod repository;
mod repository_controls;
mod repository_presentation;
mod resize;
pub(super) mod review;
#[cfg(test)]
mod tests;
pub(in crate::app::views) mod worker_list;

use gpui::{
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollAnchor, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
    uniform_list,
};

pub(super) use resize::clamped_run_panel_width;

use self::{
    agents::{AgentSection, agent_section, conversation_row},
    background_jobs::background_job_row,
    performance::render_performance,
};
use super::super::{FarcasterApp, RunPanelView};
use crate::{
    agent_activity::AgentActivity,
    app::ui::primitives::{ButtonTone, activates_button, button, panel, section_heading},
    app::ui::theme::theme,
};

pub(crate) struct RepositoryView<'a> {
    pub(crate) state: &'a change_tree::ChangeTreeState,
    pub(crate) search: &'a gpui::Entity<gpui_component::input::InputState>,
    pub(crate) query: &'a str,
    pub(crate) scroll: &'a ScrollHandle,
}

pub(super) struct WorkerListView<'a> {
    pub scroll: &'a ScrollHandle,
    pub anchor: &'a ScrollAnchor,
    pub saved_profiles: &'a WorkerProfileNames,
    pub expanded: bool,
    pub prepared: &'a std::cell::RefCell<Option<std::rc::Rc<worker_list::PreparedWorkers>>>,
    pub list: &'a gpui::UniformListScrollHandle,
    pub rows: &'a std::cell::RefCell<Vec<(String, gpui::FocusHandle)>>,
    pub main_focus: &'a gpui::FocusHandle,
    pub reveal_selected: bool,
}

pub(in crate::app) const RECENT_WORKERS: usize = 3;
pub(in crate::app) type WorkerProfileNames =
    std::collections::HashMap<crate::sessions::SessionKey, String>;

fn run_panel_agent_rows<'a>(
    sessions: &'a crate::sessions::SessionCatalog,
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    let Some(root) = sessions.root_for_path(selected) else {
        return Vec::new();
    };
    sessions
        .descendants(root)
        .into_iter()
        .filter_map(|(session, depth)| {
            let activity_key = crate::agent_activity::agent_activity_key(&session.path);
            let activity = activities
                .get(&activity_key)
                .cloned()
                .unwrap_or_else(|| AgentActivity::limited_fallback(session));
            let section = agent_section(activity.lifecycle, activity.limited, session.is_running);
            (section != AgentSection::Hidden).then_some((activity, depth, session, section))
        })
        .collect()
}

fn ordered_worker_rows<'a>(
    sessions: &'a crate::sessions::SessionCatalog,
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    let mut workers = run_panel_agent_rows(sessions, activities, selected);
    workers.sort_by(|(_, _, left, _), (_, _, right, _)| {
        (right.created_at, &right.id).cmp(&(left.created_at, &left.id))
    });
    workers
}

pub(in crate::app) fn worker_navigation_rows<'a>(
    sessions: &'a crate::sessions::SessionCatalog,
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<&'a crate::sessions::SessionSummary> {
    let Some(root) = sessions.root_for_path(selected) else {
        return Vec::new();
    };
    std::iter::once(root)
        .chain(
            ordered_worker_rows(sessions, activities, selected)
                .into_iter()
                .map(|(_, _, session, _)| session),
        )
        .collect()
}

#[cfg(test)]
pub(crate) fn live_run_panel_agent_rows<'a>(
    sessions: &'a crate::sessions::SessionCatalog,
    activities: &std::collections::HashMap<String, AgentActivity>,
    selected: Option<&std::path::Path>,
) -> Vec<(
    AgentActivity,
    usize,
    &'a crate::sessions::SessionSummary,
    AgentSection,
)> {
    run_panel_agent_rows(sessions, activities, selected)
}

impl FarcasterApp {
    fn main_agent_row(
        root: &crate::sessions::SessionSummary,
        selected: Option<&std::path::Path>,
        anchor: &ScrollAnchor,
        focus: &gpui::FocusHandle,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
    ) -> gpui::AnyElement {
        let is_selected = selected == Some(root.path.as_path());
        let path = root.path.clone();
        let project = root.project.clone();
        let app = entity.clone();
        let key_path = path.clone();
        let key_project = project.clone();
        let key_app = app.clone();
        let main_panel = run_panel.clone();
        let key_panel = run_panel.clone();
        conversation_row("run-panel-main-agent", is_selected)
            .track_focus(focus)
            .line_height(theme().type_scale.line_body)
            .debug_selector(|| "run-panel-main-agent".into())
            .anchor_scroll(is_selected.then(|| anchor.clone()))
            .aria_label("Show main agent transcript")
            .on_mouse_down(
                gpui::MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .on_click(move |_, window, cx| {
                let _ = main_panel.update(cx, |view, cx| {
                    view.collapse_workers();
                    cx.notify();
                });
                let _ = app.update(cx, |this, cx| {
                    this.select_session_and_focus(path.clone(), project.clone(), window, cx);
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let _ = key_panel.update(cx, |view, cx| {
                        view.collapse_workers();
                        cx.notify();
                    });
                    let _ = key_app.update(cx, |this, cx| {
                        this.select_session_and_focus(
                            key_path.clone(),
                            key_project.clone(),
                            window,
                            cx,
                        );
                    });
                }
            })
            .child("Main agent")
            .into_any_element()
    }

    fn expanded_worker_list(
        &self,
        prepared: &worker_list::PreparedWorkers,
        root: Option<&crate::sessions::SessionSummary>,
        selected: Option<&std::path::Path>,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
        view: &WorkerListView<'_>,
    ) -> gpui::AnyElement {
        let root = root.cloned();
        let rows = prepared.visible.clone();
        let keys = prepared.keys.clone();
        // Two worker text lines, padding, focus borders, and the 7px gap fit in every slot.
        let row_height = (theme().type_scale.line_body * 2.0).max(theme().icons.inline)
            + theme().size(15.0)
            + theme().border * 2.0;
        worker_list::reconcile(view.list, view.rows, &keys, row_height);
        if view.reveal_selected {
            let index = root
                .iter()
                .map(|session| &session.path)
                .chain(rows.iter().map(|row| &row.session.path))
                .position(|path| Some(path.as_path()) == selected);
            if let Some(index) = index {
                view.list
                    .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
            }
        }
        let selected = selected.map(std::path::Path::to_path_buf);
        let anchor = view.anchor.clone();
        let profiles = view.saved_profiles.clone();
        let has_root = root.is_some();
        let main_focus = view.main_focus.clone();
        let count = keys.len();
        let render: worker_list::RenderRow = std::rc::Rc::new(move |index, cx| {
            let row = if index == 0 && has_root {
                Self::main_agent_row(
                    root.as_ref().unwrap(),
                    selected.as_deref(),
                    &anchor,
                    &main_focus,
                    entity.clone(),
                    run_panel.clone(),
                )
            } else {
                let Some(row) = rows.get(index - usize::from(has_root)) else {
                    return div().into_any_element();
                };
                let Some(app) = entity.upgrade() else {
                    return div().into_any_element();
                };
                app.read(cx)
                    .agent_card(
                        row,
                        selected.as_deref() == Some(row.session.path.as_path()),
                        &anchor,
                        &profiles,
                        entity.clone(),
                    )
                    .unwrap_or_else(|| div().into_any_element())
            };
            div()
                .h(row_height)
                .line_height(theme().type_scale.line_body)
                .child(row)
                .into_any_element()
        });
        let visible_render = render.clone();
        uniform_list("expanded-worker-rows", count, move |range, _, cx| {
            let count = range.len();
            let started = std::time::Instant::now();
            let rows = range
                .map(|index| visible_render(index, cx))
                .collect::<Vec<_>>();
            let elapsed = started.elapsed();
            if elapsed.as_millis() >= 16 {
                zlog::warn!(
                    "PERF operation=worker_rows.build count={} elapsed_ms={:.2}",
                    count,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            rows
        })
        .track_scroll(view.list)
        .with_decoration(worker_list::FocusedRow {
            handles: keys,
            render,
        })
        .size_full()
        .into_any_element()
    }

    pub(super) fn render_run_panel(
        &self,
        entity: WeakEntity<Self>,
        run_panel: WeakEntity<RunPanelView>,
        worker_view: &WorkerListView<'_>,
        browser: &RepositoryView<'_>,
    ) -> impl IntoElement {
        let expanded = worker_view.expanded;
        let selected = self
            .lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.as_path())
            .or(self.snapshot.selected_session.as_deref());
        let root = self.sessions.all.root_for_path(selected);
        let root_path = root.map(|session| session.path.clone());
        let mut cache = worker_view.prepared.borrow_mut();
        if cache.as_ref().is_none_or(|cache| {
            cache.root != root_path || cache.generation != self.sessions.generation
        }) {
            #[cfg(test)]
            worker_list::PREPARATIONS.with(|count| count.set(count.get() + 1));
            let started = std::time::Instant::now();
            let rows: Vec<_> =
                ordered_worker_rows(&self.sessions.all, &self.activity.agents, selected)
                    .into_iter()
                    .map(|(activity, depth, session, _)| {
                        let activity_key = crate::agent_activity::agent_activity_key(&session.path);
                        std::rc::Rc::new(worker_list::WorkerRow {
                            activity,
                            depth,
                            session: session.clone(),
                            focus: self.activity.row_focus.get(&activity_key).cloned(),
                            activity_key,
                            session_key: session.key(),
                        })
                    })
                    .collect();
            let visible: Vec<_> = rows
                .iter()
                .filter(|row| row.focus.is_some())
                .cloned()
                .collect();
            let keys = root
                .iter()
                .map(|session| {
                    (
                        super::session_rail::session_row_identity(session),
                        worker_view.main_focus.clone(),
                    )
                })
                .chain(visible.iter().map(|row| {
                    (
                        format!("session:{:?}", row.session_key),
                        row.focus.clone().unwrap(),
                    )
                }))
                .collect();
            *cache = Some(std::rc::Rc::new(worker_list::PreparedWorkers {
                root: root_path.clone(),
                generation: self.sessions.generation,
                rows,
                visible: std::rc::Rc::new(visible),
                keys: std::rc::Rc::new(keys),
            }));
            let elapsed = started.elapsed();
            if elapsed.as_millis() >= 16 {
                zlog::warn!(
                    "PERF operation=worker_rows.prepare elapsed_ms={:.2}",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
        }
        let prepared = cache.as_ref().unwrap().clone();
        drop(cache);
        let workers = &prepared.rows;
        let selected_is_older = workers
            .iter()
            .skip(RECENT_WORKERS)
            .any(|row| selected == Some(row.session.path.as_path()));
        let hidden_count = workers
            .len()
            .saturating_sub(RECENT_WORKERS + usize::from(selected_is_older));
        let expand_panel = run_panel.clone();
        let conversation = if expanded {
            self.expanded_worker_list(
                &prepared,
                root,
                selected,
                entity.clone(),
                run_panel.clone(),
                worker_view,
            )
        } else {
            inspector_section()
                .when_some(root, |section, root| {
                    section.child(Self::main_agent_row(
                        root,
                        selected,
                        worker_view.anchor,
                        worker_view.main_focus,
                        entity.clone(),
                        run_panel.clone(),
                    ))
                })
                .children(
                    workers
                        .iter()
                        .enumerate()
                        .filter(|(index, row)| {
                            *index < RECENT_WORKERS || selected == Some(row.session.path.as_path())
                        })
                        .filter_map(|(_, row)| {
                            self.agent_card(
                                row,
                                selected == Some(row.session.path.as_path()),
                                worker_view.anchor,
                                worker_view.saved_profiles,
                                entity.clone(),
                            )
                        }),
                )
                .when(!expanded && hidden_count > 0, |section| {
                    section.child(
                        button(
                            "show-older-workers",
                            format!("Show more (+{hidden_count})"),
                            ButtonTone::Quiet,
                            true,
                            move |_, cx| {
                                if let Some(root) = root_path.clone() {
                                    let _ = expand_panel.update(cx, |view, cx| {
                                        view.expand_workers(root);
                                        cx.notify();
                                    });
                                }
                            },
                        )
                        .debug_selector(|| "show-more-workers".into()),
                    )
                })
                .into_any_element()
        };
        let activity = div()
            .id("run-panel-activity")
            .min_h_0()
            .when(expanded, |activity| activity.flex_1())
            .when(!expanded, |activity| {
                activity.flex_none().max_h(theme().size(320.0))
            })
            .when(!expanded, |activity| {
                activity
                    .overflow_y_scroll()
                    .track_scroll(worker_view.scroll)
            })
            .when(expanded, |activity| activity.overflow_y_hidden())
            .flex()
            .flex_col()
            .gap(theme().space.sm)
            .child(conversation)
            .when(!expanded, |activity| {
                activity.child(self.views.workgraph_sidebar.clone())
            })
            .when_some(
                self.lifecycle
                    .performance_monitor
                    .as_ref()
                    .filter(|monitor| !expanded && monitor.is_detailed()),
                |run, monitor| run.child(render_performance(&monitor.summary)),
            )
            .when(
                !expanded && !self.activity.background_jobs.is_empty(),
                |run| {
                    run.child(
                        inspector_section()
                            .child(section_heading(format!(
                                "Background jobs ({})",
                                self.activity.background_jobs.len()
                            )))
                            .children(self.activity.background_jobs.iter().map(background_job_row)),
                    )
                },
            );
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pt(theme().size(17.0))
            .pr(theme().size(15.0))
            .pb(theme().size(14.0))
            .pl(theme().size(18.0))
            .gap(theme().space.md)
            .child(activity)
            .when(
                !expanded && self.project.repository.backend.is_some(),
                |run| run.child(self.render_repository(entity.clone(), run_panel.clone(), browser)),
            );
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .bg(theme().colors.inspector)
            .child(body)
    }
}

fn inspector_section() -> gpui::Div {
    div().flex().flex_col().gap(theme().size(7.0))
}
