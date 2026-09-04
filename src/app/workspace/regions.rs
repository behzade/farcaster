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

    pub(in crate::app) fn notify_session_rail_shell(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.session_rail_view, cx);
    }

    pub(in crate::app) fn notify_archived_session_rail(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.archived_session_rail_view, cx);
    }

    pub(in crate::app) fn notify_transcript(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.transcript_view, cx);
    }

    pub(in crate::app) fn notify_composer(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.composer_view, cx);
    }

    pub(in crate::app) fn notify_run_panel(&self, cx: &mut Context<Self>) {
        Self::notify_region(&self.run_panel_view, cx);
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
