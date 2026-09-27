use super::*;

pub(super) fn model_matches(id: &str, name: &str, query: &str) -> bool {
    format!("{id} {name}").to_lowercase().contains(query)
}

fn navigation(current: usize, count: usize, key: &str) -> Option<(usize, bool)> {
    let last = count.checked_sub(1)?;
    let current = current.min(last);
    match key {
        "up" => Some((current.saturating_sub(1), false)),
        "down" => Some(((current + 1).min(last), false)),
        "enter" => Some((current, true)),
        _ => None,
    }
}

impl RuntimePickerState {
    pub(super) fn reset_results(&mut self) {
        self.highlighted = 0;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    }
}

impl FarcasterApp {
    pub(super) fn reset_model_picker_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.runtime_picker.reset_results();
        if let Some(search) = &self.workspace.runtime_picker.search {
            search.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }
}

pub(super) fn picker_panel(
    width: f32,
    height: f32,
    search_focus: gpui::FocusHandle,
    model_count: usize,
    entity: gpui::WeakEntity<FarcasterApp>,
    select: impl Fn(&mut FarcasterApp, usize, &mut Window, &mut Context<FarcasterApp>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("runtime-picker")
        .w(px(width))
        .max_h(px(height))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .bg(theme().colors.panel)
        .border(theme().border)
        .border_color(theme().colors.border)
        .rounded(theme().radius)
        .capture_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.modifiers.modified()
                || !search_focus.contains_focused(window, cx)
                || model_count == 0
            {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(key, "up" | "down" | "enter") {
                return;
            }
            window.prevent_default();
            cx.stop_propagation();
            let _ = entity.update(cx, |app, cx| {
                let Some((index, activate)) =
                    navigation(app.workspace.runtime_picker.highlighted, model_count, key)
                else {
                    return;
                };
                app.workspace.runtime_picker.highlighted = index;
                if activate {
                    select(app, index, window, cx);
                }
                app.workspace.runtime_picker.scroll.scroll_to_item(
                    app.workspace.runtime_picker.highlighted,
                    gpui::ScrollStrategy::Center,
                );
                cx.notify();
            });
        })
}

pub(super) fn result_list(
    id: &'static str,
    count: usize,
    height: f32,
    scroll: &gpui::UniformListScrollHandle,
    feedback: String,
    row: impl Fn(usize) -> gpui::AnyElement + 'static,
) -> gpui::AnyElement {
    if count == 0 {
        div()
            .p(theme().space.sm)
            .text_color(theme().colors.muted)
            .child(feedback)
            .into_any_element()
    } else {
        gpui::uniform_list(id, count, move |range, _, _| range.map(&row).collect())
            .h(px(height))
            .flex_none()
            .track_scroll(scroll)
            .into_any_element()
    }
}

pub(super) fn option_row(label: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(theme().space.sm)
        .p(theme().space.sm)
        .border_t(theme().border)
        .border_color(theme().colors.border)
        .child(div().text_color(theme().colors.muted).child(label))
}

pub(super) fn option_button(
    id: impl Into<gpui::ElementId>,
    label: String,
    selected: bool,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    button(
        id,
        label,
        if selected {
            ButtonTone::Accent
        } else {
            ButtonTone::Quiet
        },
        true,
        on_press,
    )
}

#[cfg(test)]
#[path = "mechanics_tests.rs"]
mod tests;
