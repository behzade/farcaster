use gpui::{
    App, CursorStyle, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use super::selectable_dialog_text;
use crate::{
    app::{FarcasterApp, ui::theme::THEME},
    protocol::ExtensionUiRequest,
};

#[derive(IntoElement)]
pub(super) struct SelectRequestView {
    id: String,
    title: SharedString,
    prompt: Option<SharedString>,
    options: Vec<String>,
    app: WeakEntity<FarcasterApp>,
}

impl SelectRequestView {
    pub(super) fn new(
        id: String,
        title: String,
        options: Vec<String>,
        app: WeakEntity<FarcasterApp>,
    ) -> Self {
        let (title, prompt) = dialog_copy(&title);
        Self {
            id,
            title,
            prompt,
            options,
            app,
        }
    }

    pub(super) fn title(&self) -> &SharedString {
        &self.title
    }
}

impl RenderOnce for SelectRequestView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let choices = self
            .options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let id = self.id.clone();
                let app = self.app.clone();
                let label = numbered_dialog_choice(index, &option);
                dialog_choice(
                    ("dialog-option", index),
                    label.into(),
                    index == 0,
                    move |window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.respond_dialog_value(id.clone(), option.clone(), window, cx);
                        });
                    },
                )
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.md)
            .when_some(self.prompt, |body, prompt| {
                body.child(
                    selectable_dialog_text("dialog-select-prompt", prompt)
                        .text_size(THEME.type_scale.body)
                        .text_color(THEME.colors.muted)
                        .line_height(THEME.type_scale.line_composer),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.xs)
                    .children(choices),
            )
    }
}

fn dialog_choice(
    id: impl Into<ElementId>,
    label: SharedString,
    primary: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (title, detail) = choice_copy(&label);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .w_full()
        .min_h(px(48.0))
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .px(THEME.space.sm)
        .py(THEME.space.xs)
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(if primary {
            THEME.colors.accent
        } else {
            THEME.colors.border
        })
        .bg(if primary {
            THEME.colors.selection
        } else {
            THEME.colors.surface
        })
        .hover(|choice| choice.bg(THEME.colors.hover))
        .focus(|choice| choice.border_color(THEME.colors.accent))
        .cursor(CursorStyle::PointingHand)
        .on_click(move |_, window, cx| on_press(window, cx))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .font_weight(if primary {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(THEME.colors.text)
                        .child(title),
                )
                .when_some(detail, |copy, detail| {
                    copy.child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(detail),
                    )
                }),
        )
}

pub(in crate::app::views::composer) fn dialog_copy(
    value: &str,
) -> (SharedString, Option<SharedString>) {
    let mut lines = value.lines();
    let heading = lines.next().unwrap_or_default().trim();
    let prompt = lines.collect::<Vec<_>>().join("\n");
    (
        if heading.is_empty() {
            "Action required".into()
        } else {
            heading.to_owned().into()
        },
        (!prompt.trim().is_empty()).then(|| prompt.trim().to_owned().into()),
    )
}

pub(in crate::app::views::composer) fn choice_copy(
    value: &str,
) -> (SharedString, Option<SharedString>) {
    (value.to_owned().into(), None)
}

pub(in crate::app::views::composer) fn numbered_dialog_choice(index: usize, value: &str) -> String {
    format!("{}. {value}", index + 1)
}

pub(in crate::app::views::composer) fn default_dialog_selection(
    request: &ExtensionUiRequest,
) -> Option<(&str, &str)> {
    match request {
        ExtensionUiRequest::Select { id, options, .. } => {
            options.first().map(|option| (id.as_str(), option.as_str()))
        }
        _ => None,
    }
}

pub(in crate::app::views::composer) fn dialog_number_selection<'a>(
    request: &'a ExtensionUiRequest,
    key: &str,
) -> Option<(&'a str, &'a str)> {
    let index = match key {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        "4" => 3,
        "5" => 4,
        _ => return None,
    };
    match request {
        ExtensionUiRequest::Select { id, options, .. } => options
            .get(index)
            .map(|option| (id.as_str(), option.as_str())),
        _ => None,
    }
}
