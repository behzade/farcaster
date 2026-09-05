use gpui::{
    AppContext as _, Context, Entity, IntoElement as _, Pixels, Render, ScrollHandle, Subscription,
    WeakEntity,
};
use gpui_component::input::{InputEvent, InputState};

use super::super::FarcasterApp;
use crate::app::ui::theme::THEME;

pub(crate) struct RunPanelView {
    app: WeakEntity<FarcasterApp>,
    pub(crate) changes: super::super::run_panel::change_tree::ChangeTreeState,
    search: Option<Entity<InputState>>,
    search_subscription: Option<Subscription>,
    search_project: Option<std::path::PathBuf>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    scroll: ScrollHandle,
    completed_agents_expanded: bool,
    limited_agents_expanded: bool,
}

impl RunPanelView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        Self {
            app,
            changes: Default::default(),
            search: None,
            search_subscription: None,
            search_project: None,
            width: THEME.layout.run_panel,
            resize_start: None,
            scroll: ScrollHandle::new(),
            completed_agents_expanded: false,
            limited_agents_expanded: false,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        self.width
    }

    pub(crate) fn reset_scroll(&self) {
        self.scroll
            .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
    }

    pub(crate) fn begin_resize(&mut self, pointer_x: Pixels) {
        self.resize_start = Some((pointer_x, self.width));
    }

    pub(crate) fn update_resize(&mut self, pointer_x: Pixels) -> bool {
        let Some((start_x, start_width)) = self.resize_start else {
            return false;
        };
        let width = super::super::run_panel::clamped_run_panel_width(
            f32::from(start_width) + f32::from(start_x) - f32::from(pointer_x),
        );
        if width == self.width {
            return false;
        }
        self.width = width;
        true
    }

    pub(crate) fn finish_resize(&mut self) -> bool {
        self.resize_start.take().is_some()
    }

    pub(crate) fn toggle_completed_agents(&mut self) {
        self.completed_agents_expanded = !self.completed_agents_expanded;
    }

    pub(crate) fn toggle_limited_agents(&mut self) {
        self.limited_agents_expanded = !self.limited_agents_expanded;
    }
}

impl Render for RunPanelView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.run_sidebar");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        if self.search.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files…"));
            self.search_subscription =
                Some(
                    cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.reset_scroll();
                            cx.notify();
                        }
                    }),
                );
            self.search = Some(input);
        }
        let project = app.read(cx).repository.project.clone();
        if self.search_project.as_ref() != Some(&project) {
            self.search_project = Some(project.clone());
            self.search
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.reset_scroll();
        }
        let count = app
            .read(cx)
            .repository
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.changes.len());
        self.changes.observe(&project, count);
        let search = self.search.as_ref().unwrap();
        let query = search.read(cx).value().to_string();
        app.read(cx)
            .render_run_panel(
                self.app.clone(),
                cx.entity().downgrade(),
                self.completed_agents_expanded,
                self.limited_agents_expanded,
                &super::super::run_panel::RepositoryView {
                    state: &self.changes,
                    search,
                    query: &query,
                    scroll: &self.scroll,
                },
            )
            .into_any_element()
    }
}
