use super::*;

pub(super) struct RegionViews {
    pub(super) session_rail: Entity<SessionRailView>,
    pub(super) archived_session_rail: Entity<InactiveSessionRailView>,
    pub(super) transcript: Entity<TranscriptView>,
    pub(super) composer: Entity<ComposerView>,
    pub(super) run_panel: Entity<RunPanelView>,
    pub(super) workgraph: Entity<WorkGraphBoardView>,
    pub(super) workgraph_detail: Entity<WorkGraphDetailView>,
    pub(super) workgraph_sidebar: Entity<WorkGraphSidebarView>,
}

pub(super) fn create(
    project: &Path,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> RegionViews {
    let app = cx.entity().downgrade();
    let session_rail = cx.new(|_| SessionRailView::new(app.clone()));
    let archived_session_rail =
        cx.new(|_| InactiveSessionRailView::new(app.clone(), SessionRailKind::Archived));

    let transcript_list = TranscriptListState::new();
    transcript_list.scroll_to_end();
    let transcript = cx.new(|_| TranscriptView::new(app.clone(), transcript_list.clone()));
    install_transcript_scroll_handler(&transcript_list, &transcript);

    let composer = cx.new(|_| ComposerView::new(app.clone()));
    let run_panel = cx.new(|_| RunPanelView::new(app.clone()));
    let workgraph = cx.new(|cx| {
        WorkGraphBoardView::new(
            crate::app::infrastructure::persistence::state_path(),
            project.to_path_buf(),
            window,
            cx,
        )
    });
    let workgraph_detail =
        cx.new(|cx| WorkGraphDetailView::new(app.clone(), workgraph.clone(), cx));
    let workgraph_sidebar = cx.new(|cx| {
        WorkGraphSidebarView::new(
            app,
            crate::app::infrastructure::persistence::state_path(),
            project.to_path_buf(),
            cx,
        )
    });

    RegionViews {
        session_rail,
        archived_session_rail,
        transcript,
        composer,
        run_panel,
        workgraph,
        workgraph_detail,
        workgraph_sidebar,
    }
}

fn install_transcript_scroll_handler(
    list: &TranscriptListState,
    transcript: &Entity<TranscriptView>,
) {
    let transcript = transcript.downgrade();
    list.set_scroll_handler(move |following, _, cx| {
        let needs_update = transcript.upgrade().is_some_and(|view| {
            let view = view.read(cx);
            transcript_follow_state_needs_update(view.following, view.unseen, following)
        });
        if !needs_update {
            return;
        }
        let transcript = transcript.clone();
        let deferred_at = Instant::now();
        cx.defer(move |cx| {
            crate::app::infrastructure::performance::record_scroll_defer(deferred_at.elapsed());
            let _ = transcript.update(cx, |view, cx| {
                if update_transcript_follow_state(&mut view.following, &mut view.unseen, following)
                {
                    cx.notify();
                }
            });
        });
    });
}
