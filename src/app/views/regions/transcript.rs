use std::{collections::HashMap, sync::Arc};

use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement as _, ParentElement as _, Render,
    Styled as _, WeakEntity,
};

use super::super::{FarcasterApp, transcript};
use crate::app::ui::persistent_vec::PersistentVec;

pub(crate) struct TranscriptView {
    app: WeakEntity<FarcasterApp>,
    pub(crate) focus: FocusHandle,
    markdown_cache: transcript::markdown::TranscriptMarkdownCache,
    pub(crate) font_size: gpui::Pixels,
    pub(crate) list: transcript::list::TranscriptListState,
    pub(crate) rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) disclosure_states: HashMap<usize, bool>,
    pub(crate) file_trees: HashMap<usize, crate::app::ui::change_tree::ChangeTreeState>,
    pub(crate) last_count: usize,
}

impl TranscriptView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        list: transcript::list::TranscriptListState,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
            font_size: crate::app::ui::theme::THEME.type_scale.reading,
            markdown_cache: transcript::markdown::TranscriptMarkdownCache::default(),
            list,
            rows: Arc::new(PersistentVec::default()),
            following: true,
            unseen: 0,
            disclosure_states: HashMap::new(),
            file_trees: HashMap::new(),
            last_count: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.list.reset();
        self.list.scroll_to_end();
        self.rows = Arc::new(PersistentVec::default());
        self.disclosure_states.clear();
        self.file_trees.clear();
        self.following = true;
        self.unseen = 0;
        self.last_count = 0;
    }

    pub(crate) fn apply_rows(
        &mut self,
        update: transcript::TranscriptRowUpdate,
        items: &PersistentVec<Arc<transcript::conversation::TranscriptItem>>,
    ) -> bool {
        update.apply(&self.list, &mut self.rows, items)
    }

    pub(crate) fn update_count(&mut self, count: usize) {
        if count > self.last_count && !self.following {
            self.unseen = self.unseen.saturating_add(count - self.last_count);
        }
        self.last_count = count;
    }
}

impl Render for TranscriptView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.transcript");
        let Some(app) = self.app.upgrade() else {
            return gpui::div().into_any_element();
        };
        let app = app.read(cx);
        let viewport = window.viewport_size();
        gpui::div()
            .size_full()
            .key_context(crate::app::TRANSCRIPT_KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus.focus(window, cx);
                }),
            )
            .child(transcript::render(
                &self.list,
                transcript::TranscriptViewport {
                    font_scale: self.font_size / crate::app::ui::theme::THEME.type_scale.reading,
                    following: self.following,
                    unseen: self.unseen,
                    tail_reserve: transcript::tail_reserve(viewport.height),
                },
                self.rows.clone(),
                app.snapshot.conversation.clone(),
                self.disclosure_states.clone(),
                self.file_trees.clone(),
                self.markdown_cache.clone(),
                self.app.clone(),
            ))
            .into_any_element()
    }
}
