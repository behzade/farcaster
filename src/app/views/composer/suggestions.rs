use gpui::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, StyledText, WeakEntity, Window,
    div, prelude::FluentBuilder as _,
};

use super::super::FarcasterApp;
use crate::app::{
    composer::{
        file_mentions::{self, MentionQuery},
        sessions::ComposerSnapshot,
        user_invocations::{self, ComposerSuggestion},
    },
    ui::{
        primitives::{
            ButtonTone, SelectionStyle as _, button, floating_surface, on_press_control,
            preserve_pointer_focus,
        },
        theme::theme,
    },
};

#[derive(IntoElement)]
pub(super) struct CompletionMenu {
    pub files: Option<(MentionQuery, Vec<String>)>,
    pub commands: Vec<ComposerSuggestion>,
    pub query: String,
    pub title: &'static str,
    pub empty: String,
    pub selected: usize,
    pub scroll: ScrollHandle,
    pub app: WeakEntity<FarcasterApp>,
}

impl RenderOnce for CompletionMenu {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let count = self
            .files
            .as_ref()
            .map_or(self.commands.len(), |(_, files)| files.len());
        let selected = self.selected.min(count.saturating_sub(1));
        let row_height = theme().size(28.0);
        let visible_rows = (f32::from(window.viewport_size().height * 0.4) / f32::from(row_height))
            .floor()
            .clamp(1.0, 3.0);
        let mut rows = div()
            .id("completion-results")
            .min_h_0()
            .max_h(row_height * visible_rows)
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if let Some((query, files)) = &self.files {
            for (index, path) in files.iter().enumerate() {
                let app = self.app.clone();
                let query = query.clone();
                let path = path.clone();
                let click_path = path.clone();
                let (parent, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
                let ranges = file_mentions::match_ranges(&path, &self.query);
                let offset = path.len() - name.len();
                let row = completion_row(index, selected, format!("Mention {path}"))
                    .child(
                        div()
                            .min_w_0()
                            .max_w(gpui::relative(0.6))
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(highlight(name, &ranges, offset)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(theme().colors.muted)
                            .child(highlight(parent, &ranges, 0)),
                    );
                rows = rows.child(on_press_control(row, move |window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        let (text, cursor) = file_mentions::insert(
                            &this.composer.input.read(cx).value(),
                            &query,
                            &click_path,
                        );
                        this.apply_composer_snapshot(
                            ComposerSnapshot::new(text, cursor, cursor..cursor),
                            window,
                            cx,
                        );
                        this.composer.focus.focus(window, cx);
                    });
                }));
            }
        } else {
            for (index, command) in self.commands.into_iter().enumerate() {
                let app = self.app.clone();
                let name = command.name;
                let sigil = command.sigil;
                let label = format!("{sigil}{name}");
                let ranges = file_mentions::match_ranges(&label, &self.query);
                let row = completion_row(index, selected, format!("Use {label}"))
                    .child(
                        div()
                            .min_w_0()
                            .max_w(gpui::relative(0.6))
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(highlight(&label, &ranges, 0)),
                    )
                    .when_some(command.description, |row, description| {
                        row.child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_color(theme().colors.muted)
                                .child(description),
                        )
                    });
                rows = rows.child(on_press_control(row, move |window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        let input = this.composer.input.read(cx);
                        let (text, cursor) = user_invocations::complete(
                            &input.value(),
                            input.cursor(),
                            sigil,
                            &name,
                        );
                        this.apply_composer_snapshot(
                            ComposerSnapshot::new(text, cursor, cursor..cursor),
                            window,
                            cx,
                        );
                        this.composer.focus.focus(window, cx);
                    });
                }));
            }
        }
        if count == 0 {
            rows = rows.child(
                div()
                    .px(theme().space.sm)
                    .py(theme().space.md)
                    .text_color(theme().colors.muted)
                    .child(self.empty),
            );
        }
        let retry = self.app.clone();
        floating_surface()
            .id("composer-completions")
            .debug_selector(|| "composer-completions".into())
            .role(Role::Group)
            .aria_label(self.title)
            .occlude()
            .w(theme().size(420.0))
            .max_w_full()
            .rounded(theme().radius)
            .bg(theme().colors.surface)
            .p(theme().space.xs)
            .text_size(theme().type_scale.caption)
            .text_color(theme().colors.text)
            .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
            .child(rows)
            .when(count == 0 && self.files.is_some(), |menu| {
                menu.child(button(
                    "refresh-completion-files",
                    "Refresh",
                    ButtonTone::Quiet,
                    true,
                    move |window, cx| {
                        let _ = retry.update(cx, |this, cx| {
                            this.composer.project_files_project = None;
                            this.request_composer_project_files(cx);
                            this.composer.focus.focus(window, cx);
                        });
                    },
                ))
            })
    }
}

fn completion_row(index: usize, selected: usize, label: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("completion", index))
        .role(Role::Button)
        .aria_label(label)
        .h(theme().size(28.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .px(theme().space.sm)
        .rounded(theme().radius)
        .selection_style(index == selected)
        .hover(|row| row.bg(theme().colors.highlight))
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
}

fn highlight(text: &str, ranges: &[std::ops::Range<usize>], offset: usize) -> StyledText {
    StyledText::new(text.to_owned()).with_highlights(ranges.iter().filter_map(|range| {
        let start = range.start.max(offset);
        let end = range.end.min(offset + text.len());
        (start < end).then(|| {
            (
                start - offset..end - offset,
                gpui::HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    color: Some(theme().colors.indicator.into()),
                    ..Default::default()
                },
            )
        })
    }))
}
