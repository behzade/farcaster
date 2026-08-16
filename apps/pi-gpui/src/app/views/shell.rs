use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use gpui::{
    Anchor, AnyElement, CursorStyle, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::{
    Icon, Sizable as _, Size,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::super::PiApp;
use crate::{
    assets::AppIcon,
    layout::{LayoutMode, shows_sheet_buttons},
    primitives::{ButtonTone, FeedbackTone, button, feedback, icon_button, panel, section_heading},
    projects::DraftSession,
    sessions::{
        SessionSummary, UsageSummary, descendant_sessions, root_session_for_path, root_sessions,
    },
    theme::THEME,
};

impl PiApp {
    pub(super) fn render_header(
        &self,
        mode: LayoutMode,
        entity: WeakEntity<Self>,
    ) -> impl IntoElement {
        let project_path = if self.snapshot.project.as_os_str().is_empty() {
            &self.project
        } else {
            &self.snapshot.project
        };
        let project = project_path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| self.project.display().to_string(), str::to_owned);
        let session_title =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| bounded_label(&session.title, 42))
                .unwrap_or_else(|| "New session".into());
        let sessions_entity = entity.clone();
        let run_entity = entity.clone();
        div()
            .min_h(THEME.layout.header_height)
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(THEME.space.md)
            .px(THEME.space.md)
            .py(THEME.space.xs)
            .border_b(THEME.border)
            .border_color(THEME.colors.border)
            .bg(THEME.colors.surface)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .flex_col()
                    .items_start()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .max_w(px(420.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(session_title),
                    )
                    .when(mode != LayoutMode::Narrow, |identity| {
                        identity.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(project),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .when(shows_sheet_buttons(mode), |actions| {
                        actions.child(button(
                            "open-sessions",
                            "Sessions",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ = sessions_entity
                                    .update(cx, |this, cx| this.open_sessions_sheet(window, cx));
                            },
                        ))
                    })
                    .when(shows_sheet_buttons(mode), |actions| {
                        actions.child(button(
                            "open-run",
                            "Session",
                            ButtonTone::Quiet,
                            true,
                            move |window, cx| {
                                let _ = run_entity
                                    .update(cx, |this, cx| this.open_run_sheet(window, cx));
                            },
                        ))
                    }),
            )
    }

    pub(super) fn render_sessions(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let new_entity = entity.clone();
        let add_project_entity = entity.clone();
        let current_project = if self.snapshot.project.as_os_str().is_empty() {
            self.project.clone()
        } else {
            self.snapshot.project.clone()
        };
        let mut available_projects = self.projects.clone();
        for session in &self.sessions {
            if !available_projects.contains(&session.project) {
                available_projects.push(session.project.clone());
            }
        }
        if let Some(index) = available_projects
            .iter()
            .position(|project| project == &current_project)
        {
            available_projects.swap(0, index);
        }
        let drafts = self.drafts.clone();
        let selected_draft = self.selected_draft.clone();
        let submitted_drafts = self.submitted_drafts.clone();
        let selected_root =
            root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref())
                .map(|session| session.id.clone());
        let live_root =
            root_session_for_path(&self.sessions, self.snapshot.live_session.as_deref())
                .map(|session| session.id.clone());
        let rows = session_rail_items(&self.sessions, !drafts.is_empty());
        let draft_count = drafts.len();
        let row_count = rows.len() + draft_count;
        let row_entity = entity.clone();
        let selected_live_status = self.snapshot.live_status.clone();
        let run_statuses = self.run_statuses.clone();
        let session_list = uniform_list("session-list", row_count, move |range, _, _| {
            range
                .filter_map(|index| {
                    if let Some(draft) = drafts.get(index) {
                        let selected = selected_draft.as_deref() == Some(draft.id.as_str());
                        let status = crate::app::drafts::resolved_draft_status(
                            &draft.id,
                            &submitted_drafts,
                            &run_statuses,
                        );
                        return Some(draft_session_row(
                            draft,
                            selected,
                            &status,
                            row_entity.clone(),
                        ));
                    }
                    let stored_index = index.saturating_sub(draft_count);
                    let item = rows.get(stored_index)?;
                    let selected = selected_root.as_deref() == Some(item.session.id.as_str());
                    let target = format!("session:{}", item.session.path.display());
                    let badge = run_statuses.get(&target).cloned().or_else(|| {
                        session_badge(
                            item.kind,
                            &item.session.id,
                            live_root.as_deref(),
                            &selected_live_status,
                        )
                    });
                    Some(session_row(item, selected, badge, row_entity.clone()))
                })
                .collect::<Vec<_>>()
        })
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(THEME.colors.panel)
            .child(
                div()
                    .h(THEME.layout.header_height)
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(THEME.space.md)
                    .child(
                        div()
                            .text_size(THEME.type_scale.heading)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.text)
                            .child("Pi"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(THEME.space.xs)
                            .child(
                                icon_button(
                                    "new-session",
                                    AppIcon::Plus,
                                    "New session",
                                    ButtonTone::Quiet,
                                    !available_projects.is_empty(),
                                    |_, _| {},
                                )
                                .dropdown_menu_with_anchor(
                                    Anchor::TopRight,
                                    move |menu, _, _| {
                                        let mut menu = menu
                                            .min_w(px(220.0))
                                            .max_h(px(420.0))
                                            .label("New session in");
                                        for project in &available_projects {
                                            let label = project_label(project);
                                            let target = project.clone();
                                            let entity = new_entity.clone();
                                            menu = menu.item(PopupMenuItem::new(label).on_click(
                                                move |_, window, cx| {
                                                    let _ = entity.update(cx, |this, cx| {
                                                        this.new_session(
                                                            target.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ));
                                        }
                                        menu
                                    },
                                ),
                            )
                            .child(icon_button(
                                "add-project",
                                AppIcon::FolderPlus,
                                "Add project",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let _ = add_project_entity.update(cx, |this, cx| {
                                        this.choose_project_folder(window, cx);
                                    });
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .mx(THEME.space.sm)
                    .mb(THEME.space.xs)
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .gap(THEME.space.sm)
                    .px(THEME.space.sm)
                    .rounded(THEME.radius)
                    .bg(THEME.colors.surface)
                    .text_color(THEME.colors.muted)
                    .child(Icon::new(AppIcon::MagnifyingGlass).with_size(Size::Small))
                    .child(
                        Input::new(&self.search)
                            .flex_1()
                            .min_w_0()
                            .appearance(false),
                    ),
            )
            .when_some(self.sessions_error.clone(), |rail, error| {
                rail.child(feedback("sessions-error", error, FeedbackTone::Error))
            })
            .child(
                div()
                    .id("session-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_hidden()
                    .child(session_list),
            )
            .when(row_count == 0 && self.sessions_error.is_none(), |rail| {
                rail.child(
                    div()
                        .px(THEME.space.md)
                        .py(THEME.space.sm)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child("No matching sessions"),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_run_panel(&self, entity: WeakEntity<Self>) -> impl IntoElement {
        let root = root_session_for_path(&self.sessions, self.snapshot.selected_session.as_deref());
        let descendants = root
            .map(|root| descendant_sessions(&self.sessions, &root.id))
            .unwrap_or_default();
        let show_main_context = root.is_none_or(|root| {
            self.snapshot.selected_session.as_deref() == Some(root.path.as_path())
        });
        let mut aggregate_usage = root.map(|root| root.usage).unwrap_or_default();
        for (session, _) in &descendants {
            aggregate_usage.add(session.usage);
        }
        let mut agent_rows = Vec::new();
        if let Some(root) = root {
            let root_status = if self.snapshot.live_session.as_deref() == Some(root.path.as_path())
            {
                normalized_agent_status(&self.snapshot.live_status)
            } else if root.is_running {
                "Active"
            } else {
                "Done"
            };
            agent_rows.push((
                root.clone(),
                0,
                "Main".into(),
                self.snapshot.selected_session.as_deref() == Some(root.path.as_path()),
                root_status.to_owned(),
            ));
            agent_rows.extend(descendants.into_iter().map(|(session, depth)| {
                (
                    session.clone(),
                    depth,
                    compact_subagent_label(&session.title),
                    self.snapshot.selected_session.as_deref() == Some(session.path.as_path()),
                    if session.is_running { "Active" } else { "Done" }.into(),
                )
            }));
        }
        let agent_count = agent_rows.len();
        let agent_height =
            px((agent_count.min(7) as f32) * f32::from(THEME.layout.agent_row_height));
        let agent_entity = entity.clone();
        let agent_list = uniform_list("agent-session-list", agent_count, move |range, _, _| {
            range
                .filter_map(|index| agent_rows.get(index))
                .map(|(session, depth, label, selected, status)| {
                    agent_session_row(
                        session,
                        *depth,
                        label.clone(),
                        *selected,
                        status.clone(),
                        agent_entity.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .w_full()
        .h(agent_height)
        .max_h(THEME.layout.agent_list_max_height);
        let body = div()
            .id("run-panel-scroll")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p(THEME.space.sm)
            .gap(THEME.space.md)
            .overflow_y_scroll()
            .child(section_heading("Status"))
            .child(metric_row("State", self.snapshot.status.clone()))
            .when(!self.snapshot.history_preview, |run| {
                run.child(section_heading("Model"))
                    .child(self.render_model_controls(entity.clone()))
            })
            .when_some(self.fps_monitor.clone(), |run, monitor| run.child(monitor))
            .child(section_heading("Usage"))
            .child(usage_metrics(
                show_main_context.then_some(&self.snapshot.stats),
                aggregate_usage,
                self.snapshot.conversation.latest_cache_hit_rate,
            ))
            .when(agent_count > 0, |run| {
                run.child(section_heading("Agents"))
                    .child(div().overflow_y_hidden().child(agent_list))
            });
        panel()
            .size_full()
            .rounded_none()
            .border_0()
            .child(
                div()
                    .h(THEME.layout.header_height)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.muted)
                    .child("SESSION"),
            )
            .child(body)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionRailKind {
    Project,
    Settled,
}

#[derive(Clone, Debug)]
struct SessionRailItem {
    session: SessionSummary,
    kind: SessionRailKind,
    starts_section: bool,
    starts_settled: bool,
}

fn session_rail_items(sessions: &[SessionSummary], has_drafts: bool) -> Vec<SessionRailItem> {
    let mut current = Vec::new();
    let mut settled = Vec::new();
    for session in root_sessions(sessions) {
        if !session.settled {
            current.push(SessionRailItem {
                session: session.clone(),
                kind: SessionRailKind::Project,
                starts_section: false,
                starts_settled: false,
            });
        } else {
            settled.push(SessionRailItem {
                session: session.clone(),
                kind: SessionRailKind::Settled,
                starts_section: false,
                starts_settled: false,
            });
        }
    }
    if let Some(first) = current.first_mut() {
        first.starts_section = has_drafts;
    }
    if let Some(first) = settled.first_mut() {
        first.starts_section = has_drafts || !current.is_empty();
        first.starts_settled = true;
    }
    current.extend(settled);
    current
}

fn draft_session_row(
    draft: &DraftSession,
    selected: bool,
    status: &str,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let id = draft.id.clone();
    let discard_id = id.clone();
    let project = draft.project.clone();
    let keyboard_id = id.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let discard_entity = entity.clone();
    let keyboard_discard_entity = discard_entity.clone();
    let keyboard_discard_id = discard_id.clone();
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(
            div()
                .id(format!("session-{id}"))
                .role(Role::Button)
                .aria_label(format!("Open draft session in {}", project.display()))
                .aria_selected(selected)
                .tab_index(0)
                .size_full()
                .h(THEME.layout.session_row_height)
                .flex()
                .flex_col()
                .justify_center()
                .gap(THEME.space.xs)
                .px(THEME.space.sm)
                .rounded(THEME.radius)
                .bg(if selected {
                    THEME.colors.surface
                } else {
                    THEME.colors.panel
                })
                .hover(|row| row.bg(THEME.colors.hover))
                .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
                .cursor(CursorStyle::PointingHand)
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.prevent_default();
                        let _ = keyboard_entity.update(cx, |this, cx| {
                            this.resume_draft(
                                keyboard_id.clone(),
                                keyboard_project.clone(),
                                window,
                                cx,
                            );
                        });
                    }
                })
                .on_click(move |_, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.resume_draft(id.clone(), project.clone(), window, cx);
                    });
                })
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(THEME.space.xs)
                        .text_color(THEME.colors.muted)
                        .child(Icon::new(AppIcon::Folder).with_size(Size::Small))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.caption)
                                .child(project_label(&draft.project)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(THEME.type_scale.caption)
                                .text_color(if status == "Draft" {
                                    THEME.colors.subtle
                                } else {
                                    THEME.colors.accent
                                })
                                .child(status.to_owned()),
                        )
                        .child(
                            div()
                                .id(format!("discard-{discard_id}"))
                                .role(Role::Button)
                                .aria_label("Discard draft")
                                .tab_index(0)
                                .p(THEME.space.xs)
                                .rounded(THEME.radius)
                                .hover(|button| button.bg(THEME.colors.hover))
                                .child(Icon::new(AppIcon::X).with_size(Size::Small))
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        window.prevent_default();
                                        let _ = keyboard_discard_entity.update(cx, |this, cx| {
                                            this.discard_draft(&keyboard_discard_id, window, cx);
                                        });
                                    }
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    let _ = discard_entity.update(cx, |this, cx| {
                                        this.discard_draft(&discard_id, window, cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(THEME.type_scale.body)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(THEME.colors.text)
                        .child("New session"),
                ),
        )
        .into_any_element()
}

fn session_badge(
    _kind: SessionRailKind,
    session_id: &str,
    live_session_id: Option<&str>,
    live_status: &str,
) -> Option<String> {
    if live_session_id != Some(session_id) || matches!(live_status, "" | "Done" | "Idle" | "Ready")
    {
        return None;
    }
    Some(live_status.into())
}

fn session_row(
    item: &SessionRailItem,
    selected: bool,
    status: Option<String>,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let session = &item.session;
    let path = session.path.clone();
    let project = session.project.clone();
    let keyboard_path = path.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let project_name = project_label(&project);
    let settle_path = session.path.clone();
    let settle_entity = entity.clone();
    let keyboard_settle_path = settle_path.clone();
    let keyboard_settle_entity = settle_entity.clone();
    let age = relative_age(session.modified);
    let is_settled = item.kind == SessionRailKind::Settled;
    let metadata = if item.starts_settled {
        format!("Archived · {project_name}")
    } else {
        project_name
    };
    let icon = if is_settled {
        AppIcon::ChatCircle
    } else {
        AppIcon::Folder
    };
    let status_color = match status.as_deref() {
        Some("Done") => THEME.colors.success,
        Some(_) => THEME.colors.accent,
        None => THEME.colors.subtle,
    };
    let status_text = status.unwrap_or(age);
    let settle_label = if is_settled { "Restore" } else { "Settle" };
    let row = div()
        .id(format!("session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Resume session: {}", session.title))
        .aria_selected(selected)
        .tab_index(0)
        .size_full()
        .h(THEME.layout.session_row_height)
        .flex()
        .flex_col()
        .justify_center()
        .gap(THEME.space.xs)
        .px(THEME.space.sm)
        .rounded(THEME.radius)
        .when(item.starts_section, |row| {
            row.border_t(THEME.border).border_color(THEME.colors.border)
        })
        .bg(if selected {
            THEME.colors.surface
        } else {
            THEME.colors.panel
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.resume(keyboard_path.clone(), keyboard_project.clone(), window, cx)
                });
            }
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), window, cx)
            });
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(THEME.space.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.xs)
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .text_color(if is_settled {
                                    THEME.colors.subtle
                                } else {
                                    THEME.colors.muted
                                })
                                .child(Icon::new(icon).with_size(Size::Small))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(THEME.type_scale.caption)
                                        .font_weight(if item.starts_settled {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .child(metadata),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(THEME.type_scale.body)
                                .font_weight(if selected || !is_settled {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_settled && !selected {
                                    THEME.colors.muted
                                } else {
                                    THEME.colors.text
                                })
                                .child(session.title.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(THEME.space.xs)
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(status_color)
                                .child(status_text),
                        )
                        .child(
                            div()
                                .id(format!("settle-{}", session.id))
                                .role(Role::Button)
                                .aria_label(format!("{settle_label} session"))
                                .tab_index(0)
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .px(THEME.space.xs)
                                .py(px(2.0))
                                .border(THEME.border)
                                .border_color(THEME.colors.border)
                                .rounded(THEME.radius)
                                .text_size(THEME.type_scale.caption)
                                .text_color(if is_settled {
                                    THEME.colors.success
                                } else {
                                    THEME.colors.muted
                                })
                                .hover(|button| button.bg(THEME.colors.hover))
                                .child(Icon::new(AppIcon::CheckCircle).with_size(Size::Small))
                                .child(settle_label)
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        window.prevent_default();
                                        let _ = keyboard_settle_entity.update(cx, |this, cx| {
                                            this.set_session_settled(
                                                keyboard_settle_path.clone(),
                                                !is_settled,
                                                cx,
                                            );
                                        });
                                    }
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let _ = settle_entity.update(cx, |this, cx| {
                                        this.set_session_settled(
                                            settle_path.clone(),
                                            !is_settled,
                                            cx,
                                        );
                                    });
                                }),
                        ),
                ),
        );
    div()
        .h(THEME.layout.session_row_height)
        .w_full()
        .px(THEME.space.sm)
        .child(row)
        .into_any_element()
}

fn project_label(project: &Path) -> String {
    project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| project.display().to_string(), str::to_owned)
}

fn relative_age(modified: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    if age < Duration::from_secs(60) {
        "now".into()
    } else if age < Duration::from_secs(60 * 60) {
        format!("{}m", age.as_secs() / 60)
    } else if age < Duration::from_secs(24 * 60 * 60) {
        format!("{}h", age.as_secs() / (60 * 60))
    } else {
        format!("{}d", age.as_secs() / (24 * 60 * 60))
    }
}

fn agent_session_row(
    session: &SessionSummary,
    depth: usize,
    label: String,
    selected: bool,
    status: String,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let path = session.path.clone();
    let project = session.project.clone();
    let keyboard_path = path.clone();
    let keyboard_project = project.clone();
    let keyboard_entity = entity.clone();
    let title = session.title.clone();
    let details = format!("{status} · {}", compact_number(session.usage.total));
    let status_is_active = status != "Done";
    div()
        .id(format!("agent-session-{}", session.id))
        .role(Role::Button)
        .aria_label(format!("Open agent session: {title} ({status})"))
        .aria_selected(selected)
        .tab_index(0)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .pl(px(8.0 + depth as f32 * 12.0))
        .pr(THEME.space.xs)
        .h(THEME.layout.agent_row_height)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .when(selected, |row| {
            row.border_l(px(2.0)).border_color(THEME.colors.accent)
        })
        .hover(|row| row.bg(THEME.colors.hover))
        .focus(|row| row.border(THEME.border).border_color(THEME.colors.accent))
        .cursor_pointer()
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.prevent_default();
                let _ = keyboard_entity.update(cx, |this, cx| {
                    this.resume(keyboard_path.clone(), keyboard_project.clone(), window, cx)
                });
            }
        })
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.resume(path.clone(), project.clone(), window, cx)
            });
        })
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.text)
                .child(label),
        )
        .child(
            div()
                .flex_none()
                .text_size(THEME.type_scale.caption)
                .text_color(if status_is_active {
                    THEME.colors.accent
                } else {
                    THEME.colors.subtle
                })
                .child(details),
        )
        .into_any_element()
}

fn usage_metrics(
    stats: Option<&serde_json::Value>,
    usage: UsageSummary,
    latest_cache_hit_rate: Option<f64>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(metric_row(
            "Main context",
            stats.map_or_else(|| "—".into(), context_usage),
        ))
        .child(metric_row("Tokens", compact_number(usage.total)))
        .child(metric_row("Input", compact_number(usage.input)))
        .child(metric_row("Output", compact_number(usage.output)))
        .child(metric_row(
            "Cache",
            compact_number(usage.cache_read.saturating_add(usage.cache_write)),
        ))
        .child(metric_row(
            "Cache hit rate",
            format_cache_hit_rate(latest_cache_hit_rate),
        ))
        .child(metric_row("Cost", format_cost(usage.cost_micros)))
}

fn metric_row(label: &'static str, value: String) -> impl IntoElement {
    div()
        .h(THEME.layout.status_row_height)
        .flex()
        .items_center()
        .justify_between()
        .gap(THEME.space.sm)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.subtle).child(label))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(FontWeight::MEDIUM)
                .text_color(THEME.colors.muted)
                .child(value),
        )
}

fn context_usage(stats: &serde_json::Value) -> String {
    let Some(context) = stats.get("contextUsage") else {
        return "—".into();
    };
    let tokens = context.get("tokens").and_then(serde_json::Value::as_u64);
    let window = context
        .get("contextWindow")
        .and_then(serde_json::Value::as_u64);
    let percent = context.get("percent").and_then(serde_json::Value::as_f64);
    match (tokens, window, percent) {
        (Some(tokens), Some(window), Some(percent)) => format!(
            "{} / {} · {percent:.0}%",
            compact_number(tokens),
            compact_number(window)
        ),
        (Some(tokens), Some(window), None) => {
            format!("{} / {}", compact_number(tokens), compact_number(window))
        }
        (Some(tokens), None, _) => compact_number(tokens),
        _ => "—".into(),
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        compact_scaled(value, 1_000_000, "m")
    } else if value >= 1_000 {
        compact_scaled(value, 1_000, "k")
    } else {
        value.to_string()
    }
}

fn compact_scaled(value: u64, scale: u64, suffix: &str) -> String {
    if value.is_multiple_of(scale) {
        format!("{}{suffix}", value / scale)
    } else {
        format!("{:.1}{suffix}", value as f64 / scale as f64)
    }
}

fn format_cache_hit_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "—".into(), |rate| format!("{rate:.1}%"))
}

fn format_cost(micros: u64) -> String {
    let dollars = micros as f64 / 1_000_000.0;
    if micros == 0 {
        "$0".into()
    } else if dollars < 0.01 {
        format!("${dollars:.4}")
    } else {
        format!("${dollars:.2}")
    }
}

fn bounded_label(value: &str, max: usize) -> String {
    let mut label = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        label.push('…');
    }
    label
}

fn normalized_agent_status(status: &str) -> &str {
    if matches!(status, "" | "Done" | "Idle" | "Ready") {
        "Done"
    } else {
        status
    }
}

fn compact_subagent_label(value: &str) -> String {
    let Some(generated) = value.strip_prefix("subagent-") else {
        return bounded_label(value, 24);
    };
    let Some((role, _)) = generated.split_once('-') else {
        return bounded_label(value, 24);
    };
    if role.is_empty() {
        return bounded_label(value, 24);
    }
    generated
        .rsplit('-')
        .next()
        .filter(|suffix| suffix.chars().all(|character| character.is_ascii_digit()))
        .map_or_else(|| role.to_owned(), |suffix| format!("{role} {suffix}"))
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
