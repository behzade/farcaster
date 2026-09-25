use gpui::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};

use super::super::FarcasterApp;
use crate::{
    app::composer::{
        file_mentions::{self, MentionQuery},
        sessions::ComposerSnapshot,
        user_invocations::{self, ComposerSuggestion},
    },
    app::ui::theme::{MONO_FONT_FAMILY, theme},
};

#[derive(IntoElement)]
pub(super) struct FileMentionMenu {
    files: Vec<String>,
    selected: usize,
    query: MentionQuery,
    app: WeakEntity<FarcasterApp>,
}

impl FileMentionMenu {
    pub(super) fn new(
        files: Vec<String>,
        selected: usize,
        query: MentionQuery,
        app: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            files,
            selected,
            query,
            app,
        }
    }
}

impl RenderOnce for FileMentionMenu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut menu = suggestion_menu("file-mention-menu", "Repository files");
        for (index, path) in self.files.into_iter().enumerate() {
            let app = self.app.clone();
            let query = self.query.clone();
            menu = menu.child(
                div()
                    .id(("file-mention", index))
                    .role(Role::Button)
                    .aria_label(format!("Mention {path}"))
                    .tab_index(0)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        crate::app::ui::primitives::preserve_pointer_focus,
                    )
                    .px(theme().space.sm)
                    .py(theme().space.xs)
                    .rounded(theme().radius)
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(theme().type_scale.caption)
                    .when(index == self.selected, selected_row)
                    .hover(|row| row.bg(theme().colors.highlight))
                    .focus(|row| {
                        row.border(theme().border)
                            .border_color(theme().colors.indicator)
                    })
                    .cursor_pointer()
                    .child(path.clone())
                    .on_click(move |_, window, cx| {
                        fill_file_mention(app.clone(), query.clone(), path.clone(), window, cx);
                    }),
            );
        }
        menu
    }
}

#[derive(IntoElement)]
pub(super) struct CommandMenu {
    commands: Vec<ComposerSuggestion>,
    selected: usize,
    app: WeakEntity<FarcasterApp>,
}

impl CommandMenu {
    pub(super) fn new(
        commands: Vec<ComposerSuggestion>,
        selected: usize,
        app: WeakEntity<FarcasterApp>,
    ) -> Self {
        Self {
            commands,
            selected,
            app,
        }
    }
}

impl RenderOnce for CommandMenu {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut menu = suggestion_menu("command-menu", "Commands and user invocations");
        for (index, command) in self.commands.into_iter().enumerate() {
            let name = command.name;
            let sigil = command.sigil;
            let app = self.app.clone();
            let click_name = name.clone();
            menu = menu.child(
                div()
                    .id(("composer-command", index))
                    .role(Role::Button)
                    .aria_label(format!("Use {sigil}{name}"))
                    .tab_index(0)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        crate::app::ui::primitives::preserve_pointer_focus,
                    )
                    .flex()
                    .items_center()
                    .gap(theme().space.sm)
                    .px(theme().space.sm)
                    .py(theme().space.xs)
                    .rounded(theme().radius)
                    .when(index == self.selected, selected_row)
                    .hover(|row| row.bg(theme().colors.highlight))
                    .focus(|row| {
                        row.border(theme().border)
                            .border_color(theme().colors.indicator)
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme().colors.indicator)
                            .child(format!("{sigil}{name}")),
                    )
                    .when_some(command.description, |row, description| {
                        row.child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.muted)
                                .child(description),
                        )
                    })
                    .on_click(move |_, window, cx| {
                        fill_command(app.clone(), sigil, click_name.clone(), window, cx);
                    }),
            );
        }
        menu
    }
}

fn suggestion_menu(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(label)
        .max_h(theme().size(220.0))
        .overflow_y_scroll()
        .mb(theme().space.sm)
        .border(theme().border)
        .border_color(theme().colors.border)
        .rounded(theme().radius)
        .bg(theme().colors.surface)
        .p(theme().space.xs)
}

fn selected_row(row: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    row.bg(theme().colors.highlight)
        .text_color(theme().colors.indicator)
}

fn fill_file_mention(
    entity: WeakEntity<FarcasterApp>,
    query: MentionQuery,
    path: String,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        let (text, cursor) =
            file_mentions::insert(&this.composer.input.read(cx).value(), &query, &path);
        this.apply_composer_snapshot(
            ComposerSnapshot::new(text, cursor, cursor..cursor),
            window,
            cx,
        );
        this.composer.focus.focus(window, cx);
    });
}

fn fill_command(
    entity: WeakEntity<FarcasterApp>,
    sigil: char,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        let composer = this.composer.input.read(cx);
        let (text, cursor) =
            user_invocations::complete(&composer.value(), composer.cursor(), sigil, &name);
        this.apply_composer_snapshot(
            ComposerSnapshot::new(text, cursor, cursor..cursor),
            window,
            cx,
        );
        this.composer.focus.focus(window, cx);
    });
}
