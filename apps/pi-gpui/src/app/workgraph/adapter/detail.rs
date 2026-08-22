//! Selected-issue detail presentation for the Work surface.

use gpui::{
    Anchor, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    input::{Input, Textarea},
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::WorkGraphBoardView;
use crate::{
    app::workgraph::{
        components::{
            dependency_issue_section, description_section, detail_card, notes_section,
            related_issue_section, render_edit_fields, sessions_section, status_pill,
        },
        contract::{BoardData, BoardLoadState},
        core::{
            description_copy, inspector_notes, inspector_sessions, issue_meta_label, now_millis,
            status_label,
        },
        layout::{BoardLayoutMode, DETAIL_MIN_WIDTH, DETAIL_WIDTH, issue_detail_shell},
    },
    primitives::{ButtonTone, FeedbackTone, button, dropdown_button, feedback},
    theme::THEME,
};

impl WorkGraphBoardView {
    pub(super) fn render_detail(
        &self,
        entity: Entity<Self>,
        data: &BoardData,
        layout: BoardLayoutMode,
        external: bool,
    ) -> impl IntoElement {
        let issue = self
            .selected
            .and_then(|number| data.issues.iter().find(|issue| issue.number == number));
        let narrow = issue_detail_shell(layout).shows_sheet(false);
        div()
            .id("workgraph-issue-detail")
            .when(!external, |detail| {
                detail.w(px(DETAIL_WIDTH)).min_w(px(DETAIL_MIN_WIDTH))
            })
            .when(narrow || external, |detail| detail.w_full().min_w_0())
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p(THEME.space.md)
            .bg(THEME.colors.panel)
            .when(!external, |detail| {
                detail
                    .border_l(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .child(match issue {
                Some(issue) if self.editing == Some(issue.number) => render_edit_fields(
                    issue.clone(),
                    self.edit_title.as_ref().expect("edit title initialized"),
                    self.edit_body.as_ref().expect("edit body initialized"),
                    self.edit_priority
                        .as_ref()
                        .expect("edit priority initialized"),
                    entity,
                )
                .into_any_element(),
                Some(issue) => {
                    let dependencies = data
                        .dependencies
                        .iter()
                        .filter(|edge| edge.issue_number == issue.number)
                        .filter_map(|edge| {
                            data.issues
                                .iter()
                                .find(|item| item.number == edge.depends_on)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let dependents = data
                        .dependencies
                        .iter()
                        .filter(|edge| edge.depends_on == issue.number)
                        .filter_map(|edge| {
                            data.issues
                                .iter()
                                .find(|item| item.number == edge.issue_number)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let sessions = data
                        .sessions
                        .iter()
                        .filter(|link| link.issue_number == issue.number)
                        .collect::<Vec<_>>();
                    let notes = data
                        .notes
                        .iter()
                        .filter(|note| note.issue_number == issue.number)
                        .collect::<Vec<_>>();
                    let active_link = self.active_session.as_ref().and_then(|(id, _)| {
                        data.sessions.iter().find(|link| link.session_id == *id)
                    });
                    let dependency_action = self.dependency.as_ref().map(|dependency| {
                        let dependency_input = dependency.clone();
                        let dependency_submit = dependency.clone();
                        let entity = entity.clone();
                        let number = issue.number;
                        let version = issue.version;
                        div()
                            .flex()
                            .gap(THEME.space.xs)
                            .child(Input::new(&dependency_input).w(px(160.0)))
                            .child(button(
                                format!("workgraph-add-dependency-{number}"),
                                "Add dependency",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let value =
                                        dependency_submit.read(cx).value().trim().to_owned();
                                    let Ok(depends_on) =
                                        value.strip_prefix('#').unwrap_or(&value).parse::<u64>()
                                    else {
                                        return;
                                    };
                                    dependency_submit.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx);
                                    });
                                    entity.update(cx, |this, cx| {
                                        this.change_dependency(
                                            number, depends_on, version, true, cx,
                                        );
                                    });
                                },
                            ))
                    });
                    let note_action = self.note.as_ref().map(|note| {
                        let note_input = note.clone();
                        let note_submit = note.clone();
                        let entity = entity.clone();
                        let number = issue.number;
                        let version = issue.version;
                        div()
                            .flex()
                            .flex_col()
                            .gap(THEME.space.xs)
                            .child(Textarea::new(&note_input).w_full().appearance(true))
                            .child(button(
                                format!("workgraph-add-note-{number}"),
                                "Add note",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let body = note_submit.read(cx).value().trim().to_owned();
                                    if body.is_empty() {
                                        return;
                                    }
                                    note_submit.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx);
                                    });
                                    entity.update(cx, |this, cx| {
                                        this.add_note(number, version, body, cx);
                                    });
                                },
                            ))
                    });
                    let edit_title = self
                        .edit_title
                        .as_ref()
                        .expect("edit title initialized")
                        .clone();
                    let edit_body = self
                        .edit_body
                        .as_ref()
                        .expect("edit body initialized")
                        .clone();
                    let edit_priority = self
                        .edit_priority
                        .as_ref()
                        .expect("edit priority initialized")
                        .clone();
                    let edit_entity = entity.clone();
                    let edit_number = issue.number;
                    let current_title = issue.title.clone();
                    let current_body = issue.body.clone();
                    let current_priority = issue.priority.to_string();
                    let edit_action = button(
                        format!("workgraph-edit-{edit_number}"),
                        "Edit issue",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            edit_title.update(cx, |input, cx| {
                                input.set_value(current_title.clone(), window, cx);
                            });
                            edit_body.update(cx, |input, cx| {
                                input.set_value(current_body.clone(), window, cx);
                            });
                            edit_priority.update(cx, |input, cx| {
                                input.set_value(current_priority.clone(), window, cx);
                            });
                            edit_entity.update(cx, |this, cx| {
                                this.set_editing(Some(edit_number), cx);
                            });
                        },
                    );
                    let number = issue.number;
                    let version = issue.version;
                    let current_status = issue.status;
                    let status_entity = entity.clone();
                    let status_selector = dropdown_button(
                        format!("workgraph-status-{number}"),
                        status_label(current_status),
                        ButtonTone::Neutral,
                        true,
                    )
                    .dropdown_menu_with_anchor(
                        Anchor::TopLeft,
                        move |menu, _, _| {
                            [
                                workgraph::contract::IssueStatus::Open,
                                workgraph::contract::IssueStatus::InProgress,
                                workgraph::contract::IssueStatus::Blocked,
                                workgraph::contract::IssueStatus::Done,
                                workgraph::contract::IssueStatus::Cancelled,
                            ]
                            .into_iter()
                            .filter(|status| *status != current_status)
                            .fold(
                                menu.label("Status"),
                                |menu, status| {
                                    let entity = status_entity.clone();
                                    menu.item(PopupMenuItem::new(status_label(status)).on_click(
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.set_issue_status(number, status, version, cx);
                                            });
                                        },
                                    ))
                                },
                            )
                        },
                    );
                    let session_action = self.active_session.as_ref().map(|_| {
                        let number = issue.number;
                        let entity = entity.clone();
                        let linked_here =
                            active_link.is_some_and(|link| link.issue_number == issue.number);
                        button(
                            format!("workgraph-link-session-{number}"),
                            if linked_here {
                                "Current session linked"
                            } else if active_link.is_some() {
                                "Move current session here"
                            } else {
                                "Link current session"
                            },
                            if linked_here {
                                ButtonTone::Quiet
                            } else {
                                ButtonTone::Neutral
                            },
                            !linked_here,
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.link_active_session(number, cx);
                                });
                            },
                        )
                    });
                    let back = entity.clone();
                    let now = now_millis();
                    let current_session_id =
                        self.active_session.as_ref().map(|(id, _)| id.as_str());
                    div()
                        .flex()
                        .flex_col()
                        .gap(THEME.space.sm)
                        .when(narrow, |detail| {
                            detail.child(button(
                                "workgraph-detail-back",
                                "Back to issues",
                                ButtonTone::Quiet,
                                true,
                                move |_, cx| {
                                    back.update(cx, |this, cx| this.clear_selection(cx));
                                },
                            ))
                        })
                        .child(
                            detail_card()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(THEME.type_scale.caption)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(THEME.colors.muted)
                                                .child(format!("Issue #{}", issue.number)),
                                        )
                                        .child(status_pill(issue.status)),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.display)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .line_height(THEME.type_scale.line_composer)
                                        .text_color(THEME.colors.text)
                                        .child(issue.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(THEME.type_scale.caption)
                                        .text_color(THEME.colors.subtle)
                                        .child(issue_meta_label(
                                            issue.priority,
                                            issue.updated_at,
                                            now,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(THEME.space.xs)
                                        .child(status_selector)
                                        .child(edit_action),
                                ),
                        )
                        .child(description_section(description_copy(&issue.body)))
                        .child(dependency_issue_section(
                            issue.number,
                            issue.version,
                            dependencies,
                            entity.clone(),
                            dependency_action.map(|action| action.into_any_element()),
                        ))
                        .child(related_issue_section(
                            "Unblocks",
                            "No dependent issues.",
                            dependents,
                            entity.clone(),
                        ))
                        .child(notes_section(
                            inspector_notes(&notes, now),
                            note_action.map(|action| action.into_any_element()),
                        ))
                        .child(sessions_section(
                            inspector_sessions(&sessions, current_session_id, now),
                            session_action.map(|action| action.into_any_element()),
                        ))
                        .into_any_element()
                }
                None => div()
                    .id("workgraph-detail-empty")
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(THEME.space.xs)
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(THEME.colors.muted)
                            .child("No issue selected"),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("Choose an issue to see its details and dependencies."),
                    )
                    .into_any_element(),
            })
    }

    pub(crate) fn render_external_detail(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entity = cx.entity();
        match &self.state {
            BoardLoadState::Loading => feedback(
                "workgraph-detail-loading",
                "Loading issue…",
                FeedbackTone::Info,
            )
            .into_any_element(),
            BoardLoadState::Failed(error) => {
                feedback("workgraph-detail-error", error.clone(), FeedbackTone::Error)
                    .into_any_element()
            }
            BoardLoadState::Ready(data) => self
                .render_detail(entity, data, BoardLayoutMode::Wide, true)
                .into_any_element(),
        }
    }
}
