mod attachments;
mod dialogs;
mod editor;
mod footer;
mod queue;
mod suggestions;
#[cfg(test)]
mod tests;
mod widgets;

use gpui::{
    AnyElement, App, Context, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use super::super::FarcasterApp;
use crate::{
    app::composer::{file_mentions, slash_commands, user_invocations},
    app::ui::theme::THEME,
};

#[cfg(test)]
use dialogs::{
    choice_copy, default_dialog_selection, dialog_copy, dialog_number_selection,
    numbered_dialog_choice, plain_text_html,
};
#[cfg(test)]
use queue::{QueuedMessageKind, queued_message_groups, queued_message_preview};

impl FarcasterApp {
    pub(super) fn render_composer(
        &self,
        entity: WeakEntity<Self>,
        suggestion_selection: usize,
        footer_scroll: &gpui::ScrollHandle,
        cx: &App,
    ) -> AnyElement {
        if self.extension.dialog.is_some() {
            return self.render_composer_request(entity);
        }

        let floating = self.selected_draft_is_empty_and_unsubmitted();
        let composer = self.composer.read(cx);
        let composer_text = composer.value().to_string();
        let composer_cursor = composer.cursor().min(composer_text.len());
        let composer_value = composer_text.trim().to_owned();
        let command_suggestions = slash_commands::suggestions_for_harness(
            composer_text.trim_start(),
            &self.snapshot.commands,
            self.active_harness(),
        )
        .into_iter()
        .chain(user_invocations::suggestions(
            &composer_text[..composer_cursor],
            &self.snapshot.commands,
        ))
        .take(8)
        .collect::<Vec<_>>();
        let command_suggestion_count = command_suggestions.len();
        let exact_command = slash_commands::is_exact_for_harness(
            &composer_value,
            &self.snapshot.commands,
            self.active_harness(),
        );
        let primary_action = composer_primary_action(
            !composer_value.is_empty() || self.has_composer_attachments(),
            self.can_submit(),
            exact_command,
            self.snapshot.conversation.running,
        );
        let mention_query = file_mentions::query_at_cursor(
            &self.composer.read(cx).value(),
            self.composer.read(cx).cursor(),
        );
        let file_suggestions = mention_query
            .as_ref()
            .map(|query| file_mentions::matches(&self.composer_project_files, &query.text))
            .unwrap_or_default();
        let widgets_above = widgets::render("above", &self.extension.above_widgets);
        let widgets_below = widgets::render("below", &self.extension.below_widgets);
        let mention_selection = suggestion_selection.min(file_suggestions.len().saturating_sub(1));
        let command_selection =
            suggestion_selection.min(command_suggestion_count.saturating_sub(1));
        let suggestion_count = if file_suggestions.is_empty() {
            command_suggestion_count
        } else {
            file_suggestions.len()
        };
        let actions = self.render_composer_actions(entity.clone(), primary_action);
        let input = editor::ComposerInput::new(
            self.composer.clone(),
            entity.clone(),
            suggestion_count,
            actions,
        );

        div()
            .relative()
            .w_full()
            .flex_none()
            .min_h(THEME.layout.composer_min)
            .flex()
            .flex_col()
            .when(floating, |composer| {
                composer
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .when(!floating, |composer| {
                composer
                    .border_t(THEME.border)
                    .border_color(THEME.colors.border)
            })
            .bg(THEME.colors.composer)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p(THEME.space.sm)
                    .when_some(widgets_above, |composer, widgets| composer.child(widgets))
                    .when_some(
                        queue::render(&self.snapshot.conversation.queue),
                        |composer, queue| composer.child(queue),
                    )
                    .when_some(
                        attachments::render(self, entity.clone()),
                        |composer, attachments| composer.child(attachments),
                    )
                    .when(!command_suggestions.is_empty(), |composer| {
                        composer.child(suggestions::CommandMenu::new(
                            command_suggestions,
                            command_selection,
                            entity.clone(),
                        ))
                    })
                    .when_some(
                        mention_query.filter(|_| !file_suggestions.is_empty()),
                        |composer, query| {
                            composer.child(suggestions::FileMentionMenu::new(
                                file_suggestions,
                                mention_selection,
                                query,
                                entity.clone(),
                            ))
                        },
                    )
                    .child(input)
                    .when_some(widgets_below, |composer, widgets| composer.child(widgets)),
            )
            .child(
                div()
                    .h(gpui::px(36.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(gpui::px(12.0))
                    .border_t(THEME.border)
                    .border_color(THEME.colors.surface)
                    .bg(THEME.colors.panel)
                    .child(self.render_composer_controls(entity, !floating, footer_scroll)),
            )
            .into_any_element()
    }

    fn select_previous_composer_suggestion(
        &mut self,
        suggestion_count: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        self.composer_view.update(cx, |view, cx| {
            view.select_previous_suggestion(suggestion_count, cx)
        })
    }

    fn select_next_composer_suggestion(
        &mut self,
        suggestion_count: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        self.composer_view.update(cx, |view, cx| {
            view.select_next_suggestion(suggestion_count, cx)
        })
    }
}

pub(super) fn composer_primary_action(
    has_content: bool,
    can_submit: bool,
    exact_command: bool,
    running: bool,
) -> Option<&'static str> {
    if !has_content || !can_submit {
        return None;
    }
    Some(if exact_command {
        "Run"
    } else if running {
        "Steer"
    } else {
        "Send"
    })
}
