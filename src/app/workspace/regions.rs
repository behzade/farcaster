use std::sync::Arc;

use gpui::{Context, Entity};

use super::FarcasterApp;
use crate::runtime::RuntimeCommand;

impl FarcasterApp {
    fn notify_region<V>(region: &Entity<V>, cx: &mut Context<Self>)
    where
        V: gpui::Render,
    {
        region.update(cx, |_, cx| cx.notify());
    }

    pub(in crate::app) fn notify_session_rail(&self, cx: &mut Context<Self>) {
        self.notify_session_rail_shell(cx);
        self.notify_archived_session_rail(cx);
    }

    pub(in crate::app) fn reveal_active_session_row(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
    ) {
        self.sessions.archived_expanded = false;
        self.views.session_rail.update(cx, |view, cx| {
            view.reveal = Some(key);
            cx.notify();
        });
    }

    pub(in crate::app) fn notify_session_rail_shell(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.session_rail, cx);
    }

    pub(in crate::app) fn notify_archived_session_rail(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.archived_session_rail, cx);
    }

    pub(in crate::app) fn notify_transcript(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.transcript, cx);
    }

    pub(in crate::app) fn notify_composer(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.composer, cx);
    }

    pub(in crate::app) fn notify_run_panel(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.run_panel, cx);
    }

    pub(in crate::app) fn notify_appearance(&self, cx: &mut Context<Self>) {
        self.notify_session_rail(cx);
        self.notify_composer(cx);
        self.notify_run_panel(cx);
        Self::notify_region(&self.views.workgraph, cx);
        Self::notify_region(&self.views.workgraph_detail, cx);
        Self::notify_region(&self.views.workgraph_sidebar, cx);
        self.views.transcript.update(cx, |transcript, cx| {
            transcript.list.remeasure_items(0..transcript.rows.len());
            cx.notify();
        });
    }

    pub(in crate::app) fn send(&mut self, command: RuntimeCommand, cx: &mut Context<Self>) {
        if let Err(error) = self.runtime.send(command) {
            let snapshot = Arc::make_mut(&mut self.snapshot);
            let index = snapshot.conversation.items.len();
            Arc::make_mut(&mut snapshot.conversation).push_transport_error(error);
            self.mark_transcript_changed(index, index == 0, cx);
        }
    }
}
