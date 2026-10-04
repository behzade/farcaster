use super::*;
use crate::app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward};
use gpui::Action as _;

const SECTIONS: &[&str] = &[
    "Session",
    "Configure",
    "Workspace",
    "Projects",
    "Appearance",
    "Application",
];

impl FarcasterApp {
    pub(super) fn action_picker_rows(&self) -> (Vec<PickerRow>, HashMap<String, PickerCommand>) {
        let mut commands = HashMap::new();
        let identity = self.snapshot.session_identity();
        let runtime_detail = identity.model.map(|model| {
            let mut label = format!("{} · {}", model.provider, model.name);
            if let Some(effort) = identity.effort {
                label.push_str(&format!(" · {effort}"));
            }
            label
        });
        let harness_detail = Some(if self.editable_draft_harness().is_none() {
            "Only available for drafts".into()
        } else {
            self.active_harness()
                .map(crate::agents::backend_display_name)
                .unwrap_or_else(|| "Choose a harness".into())
        });
        let sandbox_detail = Some(
            if self.snapshot.sandbox_controls_available() {
                match self.snapshot.access_mode {
                    crate::runtime::HarnessAccessMode::Full => "Off",
                    crate::runtime::HarnessAccessMode::Sandboxed => "On",
                    crate::runtime::HarnessAccessMode::Auto => "Auto",
                }
            } else {
                "Not supported by this harness"
            }
            .into(),
        );
        let mut rows = vec![
            picker_row(
                &mut commands,
                "action:new-session",
                PickerCommand::OpenProjects(ProjectPickerIntent::NewSession),
                AppIcon::Plus,
                "New session…",
                None,
                Some(application_key("n")),
                "project thread",
            )
            .section("Session"),
            picker_row(
                &mut commands,
                "action:find-session",
                PickerCommand::OpenSessions,
                AppIcon::MagnifyingGlass,
                "Find session…",
                None,
                None,
                "open resume thread",
            )
            .section("Session"),
            picker_row(
                &mut commands,
                "action:restore",
                PickerCommand::OpenScope(PickerScope::ArchivedSessions),
                AppIcon::Archive,
                "Restore session…",
                None,
                Some(application_key("shift-a")),
                "unarchive archived thread",
            )
            .section("Session"),
            picker_row(
                &mut commands,
                "action:code-task",
                PickerCommand::StartCodeTask,
                AppIcon::Code,
                "Start task from selected code…",
                (self.workspace.surface != crate::app::AppSurface::Editor)
                    .then(|| "Open an editor to select code".into()),
                Some("ctrl-g shift-n".into()),
                "neovim editor selection background new chat",
            )
            .disabled(self.workspace.surface != crate::app::AppSurface::Editor)
            .section("Session"),
            picker_row(
                &mut commands,
                "action:runtime",
                PickerCommand::OpenScope(PickerScope::Providers),
                AppIcon::List,
                "Model & effort…",
                runtime_detail,
                Some(application_key("shift-m")),
                "set provider model reasoning thinking runtime",
            )
            .section("Configure"),
            picker_row(
                &mut commands,
                "action:harness",
                PickerCommand::OpenScope(PickerScope::Harnesses),
                AppIcon::Code,
                "Harness…",
                harness_detail,
                Some(application_key("shift-h")),
                "set backend agent",
            )
            .disabled(self.editable_draft_harness().is_none())
            .section("Configure"),
            picker_row(
                &mut commands,
                "action:sandbox",
                PickerCommand::OpenScope(PickerScope::Sandbox),
                AppIcon::Shield,
                "Sandbox…",
                sandbox_detail,
                Some(application_key("shift-s")),
                "set access permissions approval",
            )
            .disabled(!self.snapshot.sandbox_controls_available())
            .section("Configure"),
            picker_row(
                &mut commands,
                "action:add-project",
                PickerCommand::AddProject(None),
                AppIcon::FolderPlus,
                "Add project",
                None,
                Some(application_key("shift-n")),
                "folder checkout",
            )
            .section("Projects"),
            picker_row(
                &mut commands,
                "action:project-work",
                PickerCommand::OpenWorkGraph,
                AppIcon::List,
                "Project work",
                Some("Open this project’s work graph".into()),
                Some(application_key("shift-i")),
                "issues tasks",
            )
            .section("Projects"),
            picker_row(
                &mut commands,
                "action:import-sessions",
                PickerCommand::ImportSessions,
                AppIcon::Binoculars,
                "Import sessions…",
                None,
                None,
                "import discover catalog disk harness",
            )
            .section("Projects"),
            picker_row(
                &mut commands,
                "action:themes",
                PickerCommand::OpenThemes,
                AppIcon::PaintRoller,
                "Themes",
                None,
                None,
                "appearance colors palette light dark editor",
            )
            .section("Appearance"),
            picker_row(
                &mut commands,
                "action:settings",
                PickerCommand::OpenSettings,
                AppIcon::GearSix,
                "Settings",
                None,
                None,
                "configuration preferences",
            )
            .section("Application"),
        ];
        let custom_actions = [
            crate::app::NewSession.name(),
            crate::app::RestoreSession.name(),
            crate::app::SetRuntime.name(),
            crate::app::SetHarness.name(),
            crate::app::SetSandbox.name(),
            crate::app::AddProject.name(),
            crate::app::ShowWorkGraph.name(),
        ];
        for command in crate::app::ui::keybindings::registry()
            .into_iter()
            .filter(|command| command.show_in_picker)
        {
            let action = command.action.name();
            if custom_actions.contains(&action) {
                continue;
            }
            let (icon, section, detail) = shortcut_presentation(action, command.section);
            let close =
                (action == crate::app::CloseCurrent.name()).then(|| self.picker_close_label());
            rows.push(
                picker_row(
                    &mut commands,
                    &format!("action:{action}"),
                    PickerCommand::Action(action),
                    icon,
                    close.unwrap_or(command.label),
                    detail.map(str::to_owned),
                    command
                        .bindings
                        .first()
                        .map(|binding| binding.keystroke.clone()),
                    command.section,
                )
                .section(section),
            );
        }

        rows.sort_by_key(|row| SECTIONS.iter().position(|section| *section == row.section));
        (rows, commands)
    }

    fn picker_close_label(&self) -> &'static str {
        use crate::app::AppSurface;
        match self.workspace.surface {
            AppSurface::Work => "Close project work",
            AppSurface::Editor => "Close editor",
            AppSurface::Terminal => "Close terminal",
            _ if self.sessions.selected_draft.is_some() => "Discard draft",
            _ if self.snapshot.selected_session.is_some() => "Archive session",
            _ => "Close current view",
        }
    }
}

fn shortcut_presentation(
    action: &str,
    fallback_section: &'static str,
) -> (AppIcon, &'static str, Option<&'static str>) {
    use crate::app::*;
    let entries = [
        (
            IncreaseTranscriptFontSize.name(),
            AppIcon::TextAa,
            "Appearance",
            None,
        ),
        (
            DecreaseTranscriptFontSize.name(),
            AppIcon::TextAa,
            "Appearance",
            None,
        ),
        (PreviousSession.name(), AppIcon::ArrowUp, "Session", None),
        (NextSession.name(), AppIcon::ArrowDown, "Session", None),
        (CloseCurrent.name(), AppIcon::X, "Session", None),
        (FocusComposer.name(), AppIcon::ChatCircle, "Workspace", None),
        (
            OpenTranscriptScratch.name(),
            AppIcon::Code,
            "Workspace",
            None,
        ),
        (ShowEditor.name(), AppIcon::Neovim, "Workspace", None),
        (
            ShowTerminal.name(),
            AppIcon::TerminalWindow,
            "Workspace",
            None,
        ),
        (
            CycleWorkspaceForward.name(),
            AppIcon::CaretRight,
            "Workspace",
            None,
        ),
        (
            CycleWorkspaceBackward.name(),
            AppIcon::ArrowLeft,
            "Workspace",
            None,
        ),
        (AbortRun.name(), AppIcon::Stop, "Session", None),
        (
            FocusSessionSearch.name(),
            AppIcon::MagnifyingGlass,
            "Session",
            Some("Focus the sidebar search field"),
        ),
        (
            ShowKeybindings.name(),
            AppIcon::Keyboard,
            "Application",
            Some("All keyboard shortcuts"),
        ),
        (
            QuitApplication.name(),
            AppIcon::SignOut,
            "Application",
            None,
        ),
    ];
    entries
        .into_iter()
        .find(|(name, ..)| *name == action)
        .map(|(_, icon, section, detail)| (icon, section, detail))
        .unwrap_or((AppIcon::Key, "Application", Some(fallback_section)))
}
