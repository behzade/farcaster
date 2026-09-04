use std::{collections::HashMap, sync::Arc};

use gpui::{Context, IntoElement as _, Render, WeakEntity};

use super::super::{FarcasterApp, transcript};
use crate::app::ui::persistent_vec::PersistentVec;

pub(crate) struct TranscriptView {
    app: WeakEntity<FarcasterApp>,
    markdown_cache: transcript::markdown::TranscriptMarkdownCache,
    pub(crate) list: transcript::list::TranscriptListState,
    pub(crate) rows: Arc<PersistentVec<transcript::TranscriptRow>>,
    pub(crate) following: bool,
    pub(crate) unseen: usize,
    pub(crate) disclosure_states: HashMap<usize, bool>,
    pub(crate) last_count: usize,
}

impl TranscriptView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        list: transcript::list::TranscriptListState,
    ) -> Self {
        Self {
            app,
            markdown_cache: transcript::markdown::TranscriptMarkdownCache::default(),
            list,
            rows: Arc::new(PersistentVec::default()),
            following: true,
            unseen: 0,
            disclosure_states: HashMap::new(),
            last_count: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.list.reset();
        self.list.scroll_to_end();
        self.rows = Arc::new(PersistentVec::default());
        self.disclosure_states.clear();
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
        transcript::render(
            &self.list,
            transcript::TranscriptViewport {
                following: self.following,
                unseen: self.unseen,
                tail_reserve: transcript::tail_reserve(viewport.height),
            },
            self.rows.clone(),
            app.snapshot.conversation.clone(),
            self.disclosure_states.clone(),
            self.markdown_cache.clone(),
            crate::agents::backend_display_name(&app.snapshot.harness).into(),
            self.app.clone(),
        )
        .into_any_element()
    }
}
