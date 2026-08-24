use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString};
use gpui_component::IconNamed;

const ICON_ROOT: &str = "icons/phosphor";
const ICON_PATHS: [&str; 36] = [
    "icons/phosphor/archive.svg",
    "icons/phosphor/arrows-clockwise.svg",
    "icons/phosphor/arrows-out.svg",
    "icons/phosphor/arrow-counter-clockwise.svg",
    "icons/phosphor/arrow-down.svg",
    "icons/phosphor/arrow-square-out.svg",
    "icons/phosphor/arrow-up.svg",
    "icons/phosphor/binoculars.svg",
    "icons/phosphor/caret-down.svg",
    "icons/phosphor/caret-right.svg",
    "icons/phosphor/chat-circle.svg",
    "icons/phosphor/chat-circle-dots.svg",
    "icons/phosphor/check-circle.svg",
    "icons/phosphor/code.svg",
    "icons/phosphor/database.svg",
    "icons/phosphor/dots-six-vertical.svg",
    "icons/phosphor/eye.svg",
    "icons/phosphor/folder.svg",
    "icons/phosphor/folder-plus.svg",
    "icons/phosphor/hammer.svg",
    "icons/phosphor/hourglass.svg",
    "icons/phosphor/key.svg",
    "icons/phosphor/list.svg",
    "icons/phosphor/magnifying-glass.svg",
    "icons/phosphor/microscope.svg",
    "icons/phosphor/plus.svg",
    "icons/phosphor/question.svg",
    "icons/phosphor/sign-in.svg",
    "icons/phosphor/spinner-gap.svg",
    "icons/phosphor/stop.svg",
    "icons/phosphor/terminal-window.svg",
    "icons/phosphor/trash.svg",
    "icons/phosphor/user-focus.svg",
    "icons/phosphor/warning-circle.svg",
    "icons/phosphor/x.svg",
    "icons/phosphor/x-circle.svg",
];

pub(crate) struct AppAssets;

impl AppAssets {
    pub(crate) fn load_fonts(&self, cx: &App) -> Result<()> {
        cx.text_system().add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Bold.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Italic.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-BoldItalic.ttf")),
            Cow::Borrowed(include_bytes!("../assets/vazirmatn/Vazirmatn-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/vazirmatn/Vazirmatn-Medium.ttf")),
            Cow::Borrowed(include_bytes!("../assets/vazirmatn/Vazirmatn-SemiBold.ttf")),
            Cow::Borrowed(include_bytes!("../assets/vazirmatn/Vazirmatn-Bold.ttf")),
        ])
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/phosphor/archive.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/archive.svg"))
            }
            "icons/phosphor/arrows-clockwise.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/arrows-clockwise.svg"
            )),
            "icons/phosphor/arrows-out.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/arrows-out.svg"))
            }
            "icons/phosphor/arrow-counter-clockwise.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/arrow-counter-clockwise.svg"
            )),
            "icons/phosphor/arrow-down.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/arrow-down.svg"))
            }
            "icons/phosphor/arrow-square-out.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/arrow-square-out.svg"
            )),
            "icons/phosphor/arrow-up.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/arrow-up.svg"))
            }
            "icons/phosphor/binoculars.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/binoculars.svg"))
            }
            "icons/phosphor/caret-down.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/caret-down.svg"))
            }
            "icons/phosphor/caret-right.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/caret-right.svg"))
            }
            "icons/phosphor/chat-circle.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/chat-circle.svg"))
            }
            "icons/phosphor/chat-circle-dots.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/chat-circle-dots.svg"
            )),
            "icons/phosphor/check-circle.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/check-circle.svg"))
            }
            "icons/phosphor/code.svg" => Some(include_bytes!("../assets/phosphor-icons/code.svg")),
            "icons/phosphor/database.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/database.svg"))
            }
            "icons/phosphor/dots-six-vertical.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/dots-six-vertical.svg"
            )),
            "icons/phosphor/eye.svg" => Some(include_bytes!("../assets/phosphor-icons/eye.svg")),
            "icons/phosphor/plus.svg" => Some(include_bytes!("../assets/phosphor-icons/plus.svg")),
            "icons/phosphor/question.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/question.svg"))
            }
            "icons/phosphor/folder.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/folder.svg"))
            }
            "icons/phosphor/folder-plus.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/folder-plus.svg"))
            }
            "icons/phosphor/git-branch.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/git-branch.svg"))
            }
            "icons/phosphor/hammer.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/hammer.svg"))
            }
            "icons/phosphor/hourglass.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/hourglass.svg"))
            }
            "icons/phosphor/key.svg" => Some(include_bytes!("../assets/phosphor-icons/key.svg")),
            "icons/phosphor/list.svg" => Some(include_bytes!("../assets/phosphor-icons/list.svg")),
            "icons/phosphor/magnifying-glass.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/magnifying-glass.svg"
            )),
            "icons/phosphor/microscope.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/microscope.svg"))
            }
            "icons/phosphor/sign-in.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/sign-in.svg"))
            }
            "icons/phosphor/spinner-gap.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/spinner-gap.svg"))
            }
            "icons/phosphor/stack.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/stack.svg"))
            }
            "icons/phosphor/stop.svg" => Some(include_bytes!("../assets/phosphor-icons/stop.svg")),
            "icons/phosphor/terminal-window.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/terminal-window.svg"
            )),
            "icons/phosphor/trash.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/trash.svg"))
            }
            "icons/phosphor/user-focus.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/user-focus.svg"))
            }
            "icons/phosphor/warning-circle.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/warning-circle.svg"
            )),
            "icons/phosphor/x.svg" => Some(include_bytes!("../assets/phosphor-icons/x.svg")),
            "icons/phosphor/x-circle.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/x-circle.svg"))
            }
            _ => None,
        };
        bytes.map_or_else(
            || gpui_component_assets::Assets.load(path),
            |bytes| Ok(Some(Cow::Borrowed(bytes))),
        )
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            ICON_PATHS
                .iter()
                .filter(|icon_path| icon_path.starts_with(path))
                .map(|icon_path| SharedString::from(*icon_path)),
        );
        Ok(assets)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppIcon {
    Archive,
    ArrowsClockwise,
    ArrowsOut,
    ArrowCounterClockwise,
    ArrowDown,
    ArrowSquareOut,
    ArrowUp,
    Binoculars,
    CaretDown,
    CaretRight,
    ChatCircle,
    ChatCircleDots,
    CheckCircle,
    Code,
    Database,
    DotsSixVertical,
    Eye,
    Folder,
    FolderPlus,
    Hammer,
    Hourglass,
    Key,
    List,
    MagnifyingGlass,
    Microscope,
    Plus,
    Question,
    SignIn,
    SpinnerGap,
    Stop,
    TerminalWindow,
    Trash,
    UserFocus,
    WarningCircle,
    X,
    XCircle,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::Archive => "archive",
            Self::ArrowsClockwise => "arrows-clockwise",
            Self::ArrowsOut => "arrows-out",
            Self::ArrowCounterClockwise => "arrow-counter-clockwise",
            Self::ArrowDown => "arrow-down",
            Self::ArrowSquareOut => "arrow-square-out",
            Self::ArrowUp => "arrow-up",
            Self::Binoculars => "binoculars",
            Self::CaretDown => "caret-down",
            Self::CaretRight => "caret-right",
            Self::ChatCircle => "chat-circle",
            Self::ChatCircleDots => "chat-circle-dots",
            Self::CheckCircle => "check-circle",
            Self::Code => "code",
            Self::Database => "database",
            Self::DotsSixVertical => "dots-six-vertical",
            Self::Eye => "eye",
            Self::Folder => "folder",
            Self::FolderPlus => "folder-plus",
            Self::Hammer => "hammer",
            Self::Hourglass => "hourglass",
            Self::Key => "key",
            Self::List => "list",
            Self::MagnifyingGlass => "magnifying-glass",
            Self::Microscope => "microscope",
            Self::Plus => "plus",
            Self::Question => "question",
            Self::SignIn => "sign-in",
            Self::SpinnerGap => "spinner-gap",
            Self::Stop => "stop",
            Self::TerminalWindow => "terminal-window",
            Self::Trash => "trash",
            Self::UserFocus => "user-focus",
            Self::WarningCircle => "warning-circle",
            Self::X => "x",
            Self::XCircle => "x-circle",
        };
        format!("{ICON_ROOT}/{name}.svg").into()
    }
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource as _;
    use gpui_component::IconNamed as _;

    use super::{AppAssets, AppIcon};

    #[test]
    fn bundled_fonts_have_true_type_headers() {
        for bytes in [
            include_bytes!("../assets/lilex/Lilex-Regular.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-Bold.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-Italic.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-BoldItalic.ttf").as_slice(),
            include_bytes!("../assets/vazirmatn/Vazirmatn-Regular.ttf").as_slice(),
            include_bytes!("../assets/vazirmatn/Vazirmatn-Medium.ttf").as_slice(),
            include_bytes!("../assets/vazirmatn/Vazirmatn-SemiBold.ttf").as_slice(),
            include_bytes!("../assets/vazirmatn/Vazirmatn-Bold.ttf").as_slice(),
        ] {
            assert!(matches!(&bytes[..4], b"\0\x01\0\0" | b"OTTO"));
        }
    }

    #[test]
    fn phosphor_icons_are_embedded_and_themeable() {
        for icon in [
            AppIcon::Archive,
            AppIcon::ArrowsClockwise,
            AppIcon::ArrowsOut,
            AppIcon::ArrowCounterClockwise,
            AppIcon::ArrowDown,
            AppIcon::ArrowSquareOut,
            AppIcon::ArrowUp,
            AppIcon::Binoculars,
            AppIcon::CaretDown,
            AppIcon::CaretRight,
            AppIcon::ChatCircle,
            AppIcon::ChatCircleDots,
            AppIcon::CheckCircle,
            AppIcon::Code,
            AppIcon::Database,
            AppIcon::DotsSixVertical,
            AppIcon::Eye,
            AppIcon::Folder,
            AppIcon::FolderPlus,
            AppIcon::Hammer,
            AppIcon::Hourglass,
            AppIcon::Key,
            AppIcon::List,
            AppIcon::MagnifyingGlass,
            AppIcon::Microscope,
            AppIcon::Plus,
            AppIcon::Question,
            AppIcon::SignIn,
            AppIcon::SpinnerGap,
            AppIcon::Stop,
            AppIcon::TerminalWindow,
            AppIcon::Trash,
            AppIcon::UserFocus,
            AppIcon::WarningCircle,
            AppIcon::X,
            AppIcon::XCircle,
        ] {
            let path = icon.path();
            let bytes = AppAssets
                .load(path.as_ref())
                .expect("asset lookup should work")
                .expect("icon should be embedded");
            assert!(bytes.starts_with(b"<svg"));
            assert!(
                bytes
                    .windows(b"currentColor".len())
                    .any(|window| window == b"currentColor")
            );
        }
    }
}
