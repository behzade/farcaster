use gpui::{Context, InteractiveElement as _};

use super::super::FarcasterApp;
use crate::app::ui::keyboard::{ClipboardCopyAlias, ClipboardPasteAlias, CopySelection};
use crate::app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward};
use crate::app::{
    AbortRun, AddProject, CloseCurrent, ComposerEscape, DismissSurface, FocusComposer,
    FocusSessionSearch, NewSession, NextSession, PickerBack, PickerScope, PreviousSession,
    ProjectPickerIntent, RemoveProject, ShowActionPicker, ShowEditor, ShowKeybindings,
    ShowTerminal, ShowWorkGraph, SubmitFollowUp, SubmitPrompt, SwitchSession0, SwitchSession1,
    SwitchSession2, SwitchSession3, SwitchSession4, SwitchSession5, SwitchSession6, SwitchSession7,
    SwitchSession8, SwitchSession9, ToggleArchivedSessions, WorkCreateIssue, WorkDismiss,
    WorkFocusSearch, WorkNextIssue, WorkPreviousIssue,
};

pub(super) fn bind(root: gpui::Div, cx: &mut Context<FarcasterApp>) -> gpui::Div {
    let root = bind_actions(root, cx);
    bind_pointer_interactions(root, cx)
}

fn bind_actions(root: gpui::Div, cx: &mut Context<FarcasterApp>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &CopySelection, window, cx| {
        this.copy_selection(window, cx);
    }))
    .on_action(cx.listener(|this, _: &ClipboardCopyAlias, window, cx| {
        this.handle_clipboard_alias(false, window, cx);
    }))
    .on_action(cx.listener(|this, _: &ClipboardPasteAlias, window, cx| {
        this.handle_clipboard_alias(true, window, cx);
    }))
    .on_action(cx.listener(|this, _: &DismissSurface, window, cx| {
        this.dismiss_surface(window, cx);
    }))
    .on_action(cx.listener(|this, _: &SubmitFollowUp, window, cx| {
        this.submit_follow_up(window, cx);
    }))
    .on_action(cx.listener(|this, _: &NewSession, window, cx| {
        this.open_picker(
            PickerScope::Projects(ProjectPickerIntent::NewSession),
            window,
            cx,
        );
    }))
    .on_action(cx.listener(|this, _: &AddProject, window, cx| {
        this.close_picker(window, cx);
        this.choose_project_folder(None, window, cx);
    }))
    .on_action(cx.listener(|this, _: &crate::app::SetSandbox, window, cx| {
        this.open_picker(PickerScope::Sandbox, window, cx);
    }))
    .on_action(cx.listener(|this, _: &crate::app::SetHarness, window, cx| {
        this.open_picker(PickerScope::Harnesses, window, cx);
    }))
    .on_action(cx.listener(|this, _: &crate::app::SetRuntime, window, cx| {
        this.open_runtime_picker(window, cx);
    }))
    .on_action(
        cx.listener(|this, _: &crate::app::RestoreSession, window, cx| {
            this.open_picker(PickerScope::ArchivedSessions, window, cx);
        }),
    )
    .on_action(cx.listener(|this, _: &ShowActionPicker, window, cx| {
        this.open_picker(PickerScope::Actions, window, cx);
    }))
    .on_action(cx.listener(|this, _: &PickerBack, window, cx| {
        this.picker_back(window, cx);
    }))
    .on_action(
        cx.listener(|this, _: &crate::app::PickerNavigateBack, window, cx| {
            this.picker_navigate_back(window, cx);
        }),
    )
    .on_action(cx.listener(|this, action: &RemoveProject, window, cx| {
        this.remove_project_from_picker(&action.path, window, cx);
    }))
    .on_action(cx.listener(|this, _: &FocusSessionSearch, window, cx| {
        this.search_focus.focus(window, cx);
    }))
    .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
        if !this.center_surface_switch_blocked() {
            this.show_chat_surface(window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &ShowEditor, window, cx| {
        this.show_editor_surface(window, cx);
    }))
    .on_action(
        cx.listener(|this, _: &crate::app::OpenTranscriptScratch, window, cx| {
            this.open_transcript_scratch(window, cx);
        }),
    )
    .on_action(cx.listener(|this, _: &ShowTerminal, window, cx| {
        this.show_terminal_surface(window, cx);
    }))
    .on_action(cx.listener(|this, _: &CycleWorkspaceForward, window, cx| {
        this.cycle_workspace_surface(true, window, cx);
    }))
    .on_action(cx.listener(|this, _: &CycleWorkspaceBackward, window, cx| {
        this.cycle_workspace_surface(false, window, cx);
    }))
    .on_action(cx.listener(|this, _: &PreviousSession, window, cx| {
        this.switch_relative_session(-1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &NextSession, window, cx| {
        this.switch_relative_session(1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &ToggleArchivedSessions, _, cx| {
        this.archived_sessions_expanded = !this.archived_sessions_expanded;
        this.notify_session_rail(cx);
    }))
    .on_action(cx.listener(|this, _: &SubmitPrompt, window, cx| {
        let value = this.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() || this.has_composer_attachments() {
            this.submit(value, this.enter_mode(), window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &AbortRun, _, cx| {
        if this.snapshot.conversation.running {
            this.send(crate::runtime::RuntimeCommand::Abort, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &ComposerEscape, _, cx| {
        this.handle_composer_escape(cx);
    }))
    .on_action(cx.listener(|this, _: &CloseCurrent, window, cx| {
        this.close_current_target(window, cx);
    }))
    .on_action(cx.listener(|this, _: &ShowKeybindings, window, cx| {
        this.open_keybindings_help(window, cx);
    }))
    .on_action(cx.listener(|this, _: &ShowWorkGraph, window, cx| {
        this.toggle_workgraph_surface(window, cx);
    }))
    .on_action(cx.listener(|this, _: &WorkPreviousIssue, _, cx| {
        this.workgraph_view
            .update(cx, |view, cx| view.move_selection(-1, cx));
    }))
    .on_action(cx.listener(|this, _: &WorkNextIssue, _, cx| {
        this.workgraph_view
            .update(cx, |view, cx| view.move_selection(1, cx));
    }))
    .on_action(cx.listener(|this, _: &WorkFocusSearch, window, cx| {
        this.workgraph_view
            .update(cx, |view, cx| view.focus_search(window, cx));
    }))
    .on_action(cx.listener(|this, _: &WorkCreateIssue, window, cx| {
        this.workgraph_view
            .update(cx, |view, cx| view.start_create(window, cx));
    }))
    .on_action(cx.listener(|this, _: &crate::app::WorkBack, window, cx| {
        this.workgraph_view
            .update(cx, |view, cx| view.back_to_plans(window, cx));
    }))
    .on_action(cx.listener(|this, _: &WorkDismiss, window, cx| {
        let handled = this
            .workgraph_view
            .update(cx, |view, cx| view.dismiss_work_state(window, cx));
        if !handled {
            this.show_chat_surface(window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &SwitchSession0, window, cx| {
        this.switch_to_first_unsubmitted_draft(window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession1, window, cx| {
        this.switch_to_session_number(1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession2, window, cx| {
        this.switch_to_session_number(2, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession3, window, cx| {
        this.switch_to_session_number(3, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession4, window, cx| {
        this.switch_to_session_number(4, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession5, window, cx| {
        this.switch_to_session_number(5, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession6, window, cx| {
        this.switch_to_session_number(6, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession7, window, cx| {
        this.switch_to_session_number(7, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession8, window, cx| {
        this.switch_to_session_number(8, window, cx);
    }))
    .on_action(cx.listener(|this, _: &SwitchSession9, window, cx| {
        this.switch_to_session_number(9, window, cx);
    }))
}

fn bind_pointer_interactions(root: gpui::Div, cx: &mut Context<FarcasterApp>) -> gpui::Div {
    root.on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
        this.update_session_rail_resize(event.position.x, cx);
        this.update_run_panel_resize(event.position.x, cx);
    }))
    .on_mouse_up(
        gpui::MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.finish_session_rail_resize(cx);
            this.finish_run_panel_resize(cx);
        }),
    )
    .on_mouse_up_out(
        gpui::MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.finish_session_rail_resize(cx);
            this.finish_run_panel_resize(cx);
        }),
    )
}
