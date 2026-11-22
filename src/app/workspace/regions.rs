use std::sync::Arc;

use gpui::{Context, Entity};

use super::FarcasterApp;
use crate::app::views::SessionReveal;
use crate::runtime::RuntimeCommand;

impl FarcasterApp {
    fn notify_region<V>(region: &Entity<V>, cx: &mut Context<Self>)
    where
        V: gpui::Render,
    {
        region.update(cx, |_, cx| cx.notify());
    }

    pub(in crate::app) fn notify_session_rail(&self, cx: &mut Context<Self>) {
        self.invalidate_active_rail_rows(cx);
        self.notify_archived_session_rail(cx);
    }

    pub(in crate::app) fn reveal_active_session_row(
        &mut self,
        key: String,
        cx: &mut Context<Self>,
    ) {
        self.sessions.archived_expanded = false;
        self.views.session_rail.update(cx, |view, cx| {
            view.reveal = Some(SessionReveal::SessionID(key));
            cx.notify();
        });
    }

    pub(in crate::app) fn invalidate_active_rail_rows(&self, cx: &mut Context<Self>) {
        self.views.session_rail.update(cx, |rail, cx| {
            rail.invalidate_rows();
            cx.notify();
        });
    }

    pub(in crate::app) fn notify_active_archive_change(
        &self,
        key: Option<&crate::sessions::SessionKey>,
        draft: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.views.session_rail.update(cx, |rail, cx| {
            rail.patch_archive(self, key, draft);
            cx.notify();
        });
    }

    pub(in crate::app) fn notify_session_rail_shell(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.views.session_rail, cx);
    }

    pub(in crate::app) fn notify_archived_session_rail(&self, cx: &mut Context<Self>) {
        self.views.archived_session_rail.update(cx, |rail, cx| {
            rail.invalidate_rows();
            cx.notify();
        });
    }

    pub(in crate::app) fn repaint_archived_session_rail(&self, cx: &mut Context<Self>) {
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
        if matches!(
            &command,
            RuntimeCommand::SetModel(_)
                | RuntimeCommand::SetModelWithAccessMode { .. }
                | RuntimeCommand::SetThinking(_)
                | RuntimeCommand::ResetThinking
                | RuntimeCommand::SetServiceTier(_)
                | RuntimeCommand::SetAccessMode(_)
        ) {
            if !self.can_submit() {
                return;
            }
            self.engage_current_draft();
        }
        let selection = self.record_selection_dispatch(&command);
        if let Err(error) = self.runtime.send(command) {
            if let Some(request) = selection {
                self.finish_selection_route(request, false, cx);
            }
            let snapshot = Arc::make_mut(&mut self.snapshot);
            let index = snapshot.conversation.items.len();
            Arc::make_mut(&mut snapshot.conversation).push_transport_error(error);
            self.mark_transcript_changed(index, index == 0, cx);
        }
    }
}
