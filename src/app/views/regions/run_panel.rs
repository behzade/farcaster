use gpui::{
    AppContext as _, Context, Entity, IntoElement as _, Pixels, Render, ScrollAnchor, ScrollHandle,
    Subscription, WeakEntity,
};
use gpui_component::input::{InputEvent, InputState};

use super::super::FarcasterApp;
use crate::app::ui::theme::theme;

pub(crate) struct RunPanelView {
    app: WeakEntity<FarcasterApp>,
    pub(crate) changes: super::super::run_panel::change_tree::ChangeTreeState,
    search: Option<Entity<InputState>>,
    search_subscription: Option<Subscription>,
    search_project: Option<std::path::PathBuf>,
    width: Pixels,
    resize_start: Option<(Pixels, Pixels)>,
    scroll: ScrollHandle,
    activity_scroll: ScrollHandle,
    activity_anchor: ScrollAnchor,
    review_scroll: ScrollHandle,
    pub(crate) review_tree: super::super::run_panel::change_tree::ChangeTreeState,
    review_id: Option<u64>,
    expanded_workers_root: Option<std::path::PathBuf>,
    last_selected: Option<std::path::PathBuf>,
    reveal_selected: bool,
    worker_profiles_generation: Option<u64>,
    saved_worker_profiles: super::super::run_panel::WorkerProfileNames,
}

impl RunPanelView {
    pub(crate) fn new(app: WeakEntity<FarcasterApp>) -> Self {
        let activity_scroll = ScrollHandle::new();
        Self {
            app,
            changes: Default::default(),
            search: None,
            search_subscription: None,
            search_project: None,
            width: theme().layout.run_panel,
            resize_start: None,
            scroll: ScrollHandle::new(),
            activity_anchor: ScrollAnchor::for_handle(activity_scroll.clone()),
            activity_scroll,
            review_scroll: ScrollHandle::new(),
            review_tree: Default::default(),
            review_id: None,
            expanded_workers_root: None,
            last_selected: None,
            reveal_selected: false,
            worker_profiles_generation: None,
            saved_worker_profiles: Default::default(),
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

    pub(crate) fn expand_workers(&mut self, root: std::path::PathBuf) {
        self.expanded_workers_root = Some(root);
        self.reveal_selected = false;
    }

    pub(crate) fn collapse_workers(&mut self) {
        self.expanded_workers_root = None;
        self.reveal_selected = true;
    }

    pub(crate) fn workers_expanded_for(&self, root: &std::path::Path) -> bool {
        self.expanded_workers_root.as_deref() == Some(root)
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
        let generation = app.read(cx).sessions.generation;
        if self.worker_profiles_generation != Some(generation) {
            self.worker_profiles_generation = Some(generation);
            if let Ok(families) =
                crate::app::persistence::open().and_then(|store| store.load_worker_routes())
            {
                self.saved_worker_profiles = families
                    .into_iter()
                    .filter_map(|family| {
                        let profile = family.routing?.assignment.profile;
                        Some((
                            (family.project, family.child_backend, family.child_session),
                            profile,
                        ))
                    })
                    .collect();
            }
        }
        if let Some(review) = app.read(cx).visible_review() {
            if self.review_id != Some(review.id) {
                self.review_id = Some(review.id);
                self.review_tree = Default::default();
                self.review_scroll
                    .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
            }
            return super::super::run_panel::review::render(
                review,
                &self.review_scroll,
                &self.review_tree,
                self.app.clone(),
                cx.entity().downgrade(),
            );
        }
        let selected = app
            .read(cx)
            .lifecycle
            .pending_session_switch
            .as_ref()
            .map(|(path, _)| path.clone())
            .or_else(|| app.read(cx).snapshot.selected_session.clone());
        let root = app
            .read(cx)
            .sessions
            .all
            .root_for_path(selected.as_deref())
            .map(|session| session.path.clone());
        if self.expanded_workers_root.as_ref() != root.as_ref()
            || app.read(cx).sessions.selected_draft.is_some()
            || (self.last_selected != selected && selected == root)
        {
            self.expanded_workers_root = None;
        }
        if self.last_selected != selected || self.reveal_selected {
            self.last_selected = selected;
            self.reveal_selected = false;
            if root.is_some() && self.last_selected.is_some() {
                self.activity_anchor.scroll_to(window, cx);
            }
        }
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
        let project = app.read(cx).project.repository.project.clone();
        if self.search_project.as_ref() != Some(&project) {
            self.search_project = Some(project.clone());
            self.search
                .as_ref()
                .expect("search input initialized above")
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.reset_scroll();
        }
        let count = app
            .read(cx)
            .project
            .repository
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.changes.len());
        self.changes.observe(&project, count);
        let search = self
            .search
            .as_ref()
            .expect("search input initialized above");
        let query = search.read(cx).value().to_string();
        app.read(cx)
            .render_run_panel(
                self.app.clone(),
                cx.entity().downgrade(),
                &super::super::run_panel::WorkerListView {
                    scroll: &self.activity_scroll,
                    anchor: &self.activity_anchor,
                    saved_profiles: &self.saved_worker_profiles,
                    expanded: self.expanded_workers_root.is_some(),
                },
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
