use gpui::{
    App, AppContext as _, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions,
};
use gpui_component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};

use crate::app::{
    AddProject, CloseCurrent, NewSession, QuitApplication, ShowActionPicker, ShowEditor,
    ShowKeybindings, ShowTerminal, ShowWorkGraph,
};

actions!(
    farcaster,
    [
        HideApplication,
        HideOtherApplications,
        ShowAllApplications,
        CloseWindow,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        BringAllToFront,
    ]
);

fn with_active_window(cx: &mut App, action: impl FnOnce(&mut Window)) {
    if let Some(window) = cx.active_window() {
        let _ = cx.update_window(window, |_, window, _| action(window));
    }
}

pub(super) fn install(cx: &mut App) {
    cx.on_action(|_: &HideApplication, cx| cx.hide());
    cx.on_action(|_: &HideOtherApplications, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApplications, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| cx.dispatch_action(&QuitApplication));
    cx.on_action(|_: &MinimizeWindow, cx| {
        with_active_window(cx, |window| window.minimize_window());
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        with_active_window(cx, |window| window.zoom_window());
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        with_active_window(cx, |window| window.toggle_fullscreen());
    });
    cx.on_action(|_: &BringAllToFront, cx| {
        let active = cx.active_window();
        cx.activate(true);
        for window in cx.windows() {
            let _ = cx.update_window(window, |_, window, _| window.activate_window());
        }
        if let Some(window) = active {
            let _ = cx.update_window(window, |_, window, _| window.activate_window());
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-h", HideApplication, None),
        KeyBinding::new("alt-cmd-h", HideOtherApplications, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-shift-w", CloseWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    cx.set_menus(vec![
        Menu::new("Farcaster").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Farcaster", HideApplication),
            MenuItem::action("Hide Others", HideOtherApplications),
            MenuItem::action("Show All", ShowAllApplications),
            MenuItem::separator(),
            MenuItem::action("Quit Farcaster", QuitApplication),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Session", NewSession),
            MenuItem::action("Add Project…", AddProject),
            MenuItem::separator(),
            MenuItem::action("Close Surface or Archive Session", CloseCurrent),
            // Closing the sole app window exits, so retain the active-work prompt.
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Action Picker…", ShowActionPicker),
            MenuItem::action("Editor", ShowEditor),
            MenuItem::action("Terminal", ShowTerminal),
            MenuItem::action("Work Graph", ShowWorkGraph),
        ]),
        // GPUI registers this name as the native Windows menu, allowing macOS
        // to add its own window navigation and management items.
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
        Menu::new("Help").items([MenuItem::action("Keyboard Shortcuts", ShowKeybindings)]),
    ]);
}
