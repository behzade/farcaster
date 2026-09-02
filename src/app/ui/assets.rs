use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString};
use gpui_component::IconNamed;

const ICON_ROOT: &str = "icons/phosphor";
const ICON_PATHS: [&str; 49] = [
    "icons/phosphor/archive.svg",
    "icons/phosphor/arrows-clockwise.svg",
    "icons/phosphor/arrows-out.svg",
    "icons/phosphor/arrow-counter-clockwise.svg",
    "icons/phosphor/arrow-down.svg",
    "icons/phosphor/arrow-square-out.svg",
    "icons/phosphor/arrow-up.svg",
    "icons/phosphor/binoculars.svg",
    "icons/phosphor/caret-down.svg",
    "icons/phosphor/caret-left.svg",
    "icons/phosphor/caret-right.svg",
    "icons/phosphor/chat-circle.svg",
    "icons/phosphor/chat-circle-dots.svg",
    "icons/phosphor/check.svg",
    "icons/phosphor/check-circle.svg",
    "icons/phosphor/code.svg",
    "icons/phosphor/dots-six-vertical.svg",
    "icons/phosphor/eye.svg",
    "icons/phosphor/eye-slash.svg",
    "icons/phosphor/folder.svg",
    "icons/phosphor/folder-plus.svg",
    "icons/phosphor/git-fork.svg",
    "icons/phosphor/globe.svg",
    "icons/phosphor/hammer.svg",
    "icons/phosphor/hourglass.svg",
    "icons/phosphor/info.svg",
    "icons/phosphor/key.svg",
    "icons/phosphor/list.svg",
    "icons/phosphor/magnifying-glass.svg",
    "icons/phosphor/microscope.svg",
    "icons/phosphor/plus.svg",
    "icons/phosphor/question.svg",
    "icons/phosphor/shield.svg",
    "icons/phosphor/sign-in.svg",
    "icons/phosphor/spinner-gap.svg",
    "icons/phosphor/stop.svg",
    "icons/phosphor/terminal-window.svg",
    "icons/phosphor/text-aa.svg",
    "icons/phosphor/trash.svg",
    "icons/phosphor/tray.svg",
    "icons/phosphor/user-focus.svg",
    "icons/phosphor/warning-circle.svg",
    "icons/phosphor/x.svg",
    "icons/phosphor/x-circle.svg",
    "icons/workbench/codex.svg",
    "icons/workbench/ghostty.svg",
    "icons/workbench/neovim.svg",
    "icons/workbench/opencode.svg",
    "icons/workbench/pi.svg",
];

pub(crate) struct AppAssets;

impl AppAssets {
    pub(crate) fn load_fonts(&self, cx: &App) -> Result<()> {
        cx.text_system().add_fonts(vec![
            Cow::Borrowed(include_bytes!("../../../assets/lilex/Lilex-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../../../assets/lilex/Lilex-Bold.ttf")),
            Cow::Borrowed(include_bytes!("../../../assets/lilex/Lilex-Italic.ttf")),
            Cow::Borrowed(include_bytes!("../../../assets/lilex/Lilex-BoldItalic.ttf")),
            Cow::Borrowed(include_bytes!(
                "../../../assets/vazirmatn/Vazirmatn-Regular.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../../../assets/vazirmatn/Vazirmatn-Medium.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../../../assets/vazirmatn/Vazirmatn-SemiBold.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../../../assets/vazirmatn/Vazirmatn-Bold.ttf"
            )),
        ])
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/phosphor/archive.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/archive.svg"))
            }
            "icons/phosphor/arrows-clockwise.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrows-clockwise.svg"
            )),
            "icons/phosphor/arrows-out.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrows-out.svg"
            )),
            "icons/phosphor/arrow-counter-clockwise.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrow-counter-clockwise.svg"
            )),
            "icons/phosphor/arrow-down.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrow-down.svg"
            )),
            "icons/phosphor/arrow-square-out.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrow-square-out.svg"
            )),
            "icons/phosphor/arrow-up.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/arrow-up.svg"
            )),
            "icons/phosphor/binoculars.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/binoculars.svg"
            )),
            "icons/phosphor/caret-down.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/caret-down.svg"
            )),
            "icons/phosphor/caret-left.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/caret-left.svg"
            )),
            "icons/phosphor/caret-right.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/caret-right.svg"
            )),
            "icons/phosphor/chat-circle.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/chat-circle.svg"
            )),
            "icons/phosphor/chat-circle-dots.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/chat-circle-dots.svg"
            )),
            "icons/phosphor/check.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/check.svg"))
            }
            "icons/phosphor/check-circle.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/check-circle.svg"
            )),
            "icons/phosphor/code.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/code.svg"))
            }
            "icons/phosphor/dots-six-vertical.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/dots-six-vertical.svg"
            )),
            "icons/phosphor/eye.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/eye.svg"))
            }
            "icons/phosphor/eye-slash.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/eye-slash.svg"
            )),
            "icons/phosphor/plus.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/plus.svg"))
            }
            "icons/phosphor/question.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/question.svg"
            )),
            "icons/phosphor/folder.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/folder.svg"))
            }
            "icons/phosphor/folder-plus.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/folder-plus.svg"
            )),
            "icons/phosphor/globe.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/globe.svg"))
            }
            "icons/phosphor/git-fork.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/git-fork.svg"
            )),
            "icons/phosphor/git-branch.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/git-branch.svg"
            )),
            "icons/phosphor/hammer.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/hammer.svg"))
            }
            "icons/phosphor/hourglass.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/hourglass.svg"
            )),
            "icons/phosphor/info.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/info.svg"))
            }
            "icons/phosphor/key.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/key.svg"))
            }
            "icons/phosphor/list.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/list.svg"))
            }
            "icons/phosphor/magnifying-glass.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/magnifying-glass.svg"
            )),
            "icons/phosphor/microscope.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/microscope.svg"
            )),
            "icons/phosphor/shield.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/shield.svg"))
            }
            "icons/phosphor/sign-in.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/sign-in.svg"))
            }
            "icons/phosphor/spinner-gap.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/spinner-gap.svg"
            )),
            "icons/phosphor/stack.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/stack.svg"))
            }
            "icons/phosphor/stop.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/stop.svg"))
            }
            "icons/phosphor/terminal-window.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/terminal-window.svg"
            )),
            "icons/phosphor/text-aa.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/text-aa.svg"))
            }
            "icons/phosphor/trash.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/trash.svg"))
            }
            "icons/phosphor/tray.svg" => {
                Some(include_bytes!("../../../assets/phosphor-icons/tray.svg"))
            }
            "icons/phosphor/user-focus.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/user-focus.svg"
            )),
            "icons/phosphor/warning-circle.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/warning-circle.svg"
            )),
            "icons/phosphor/x.svg" => Some(include_bytes!("../../../assets/phosphor-icons/x.svg")),
            "icons/phosphor/x-circle.svg" => Some(include_bytes!(
                "../../../assets/phosphor-icons/x-circle.svg"
            )),
            "icons/workbench/codex.svg" => {
                Some(include_bytes!("../../../assets/workbench-icons/codex.svg"))
            }
            "icons/workbench/ghostty.svg" => Some(include_bytes!(
                "../../../assets/workbench-icons/ghostty.svg"
            )),
            "icons/workbench/neovim.svg" => {
                Some(include_bytes!("../../../assets/workbench-icons/neovim.svg"))
            }
            "icons/workbench/opencode.svg" => Some(include_bytes!(
                "../../../assets/workbench-icons/opencode.svg"
            )),
            "icons/workbench/pi.svg" => {
                Some(include_bytes!("../../../assets/workbench-icons/pi.svg"))
            }
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICON_PATHS
            .iter()
            .filter(|icon_path| icon_path.starts_with(path))
            .map(|icon_path| SharedString::from(*icon_path))
            .collect())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppIcon {
    Archive,
    ArrowsClockwise,
    ArrowsOut,
    ArrowCounterClockwise,
    ArrowDown,
    ArrowUp,
    Binoculars,
    CaretDown,
    CaretRight,
    ChatCircle,
    ChatCircleDots,
    CheckCircle,
    Code,
    Codex,
    Eye,
    Folder,
    FolderPlus,
    Ghostty,
    GitFork,
    Hammer,
    Hourglass,
    List,
    MagnifyingGlass,
    Microscope,
    Neovim,
    OpenCode,
    Pi,
    Plus,
    Question,
    Shield,
    SpinnerGap,
    Stop,
    Trash,
    UserFocus,
    WarningCircle,
    X,
    XCircle,
}

impl AppIcon {
    pub(crate) fn for_harness(harness: &str) -> Self {
        match harness {
            "pi" => Self::Pi,
            "codex-cli" => Self::Codex,
            "opencode2" => Self::OpenCode,
            _ => Self::Code,
        }
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::Archive => "archive",
            Self::ArrowsClockwise => "arrows-clockwise",
            Self::ArrowsOut => "arrows-out",
            Self::ArrowCounterClockwise => "arrow-counter-clockwise",
            Self::ArrowDown => "arrow-down",
            Self::ArrowUp => "arrow-up",
            Self::Binoculars => "binoculars",
            Self::CaretDown => "caret-down",
            Self::CaretRight => "caret-right",
            Self::ChatCircle => "chat-circle",
            Self::ChatCircleDots => "chat-circle-dots",
            Self::CheckCircle => "check-circle",
            Self::Code => "code",
            Self::Codex => return "icons/workbench/codex.svg".into(),
            Self::Eye => "eye",
            Self::Folder => "folder",
            Self::FolderPlus => "folder-plus",
            Self::Ghostty => return "icons/workbench/ghostty.svg".into(),
            Self::GitFork => "git-fork",
            Self::Hammer => "hammer",
            Self::Hourglass => "hourglass",
            Self::List => "list",
            Self::MagnifyingGlass => "magnifying-glass",
            Self::Microscope => "microscope",
            Self::Neovim => return "icons/workbench/neovim.svg".into(),
            Self::OpenCode => return "icons/workbench/opencode.svg".into(),
            Self::Pi => return "icons/workbench/pi.svg".into(),
            Self::Plus => "plus",
            Self::Question => "question",
            Self::Shield => "shield",
            Self::SpinnerGap => "spinner-gap",
            Self::Stop => "stop",
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
    use gpui_component::{IconName, IconNamed as _};

    use super::{AppAssets, AppIcon};

    #[test]
    fn bundled_fonts_have_true_type_headers() {
        for bytes in [
            include_bytes!("../../../assets/lilex/Lilex-Regular.ttf").as_slice(),
            include_bytes!("../../../assets/lilex/Lilex-Bold.ttf").as_slice(),
            include_bytes!("../../../assets/lilex/Lilex-Italic.ttf").as_slice(),
            include_bytes!("../../../assets/lilex/Lilex-BoldItalic.ttf").as_slice(),
            include_bytes!("../../../assets/vazirmatn/Vazirmatn-Regular.ttf").as_slice(),
            include_bytes!("../../../assets/vazirmatn/Vazirmatn-Medium.ttf").as_slice(),
            include_bytes!("../../../assets/vazirmatn/Vazirmatn-SemiBold.ttf").as_slice(),
            include_bytes!("../../../assets/vazirmatn/Vazirmatn-Bold.ttf").as_slice(),
        ] {
            assert!(matches!(&bytes[..4], b"\0\x01\0\0" | b"OTTO"));
        }
    }

    #[test]
    fn harnesses_use_their_brand_icons() {
        assert_eq!(AppIcon::for_harness("pi"), AppIcon::Pi);
        assert_eq!(AppIcon::for_harness("codex-cli"), AppIcon::Codex);
        assert_eq!(AppIcon::for_harness("opencode2"), AppIcon::OpenCode);
        assert_eq!(AppIcon::for_harness("unknown"), AppIcon::Code);
    }

    #[test]
    fn asset_source_serves_only_themeable_icons() {
        assert!(
            AppAssets
                .load("icons/search.svg")
                .expect("asset lookup should work")
                .is_none()
        );

        for icon in [
            AppIcon::Archive,
            AppIcon::ArrowsClockwise,
            AppIcon::ArrowsOut,
            AppIcon::ArrowCounterClockwise,
            AppIcon::ArrowDown,
            AppIcon::ArrowUp,
            AppIcon::Binoculars,
            AppIcon::CaretDown,
            AppIcon::CaretRight,
            AppIcon::ChatCircle,
            AppIcon::ChatCircleDots,
            AppIcon::CheckCircle,
            AppIcon::Code,
            AppIcon::Codex,
            AppIcon::Eye,
            AppIcon::Folder,
            AppIcon::FolderPlus,
            AppIcon::Ghostty,
            AppIcon::GitFork,
            AppIcon::Hammer,
            AppIcon::Hourglass,
            AppIcon::List,
            AppIcon::MagnifyingGlass,
            AppIcon::Microscope,
            AppIcon::Neovim,
            AppIcon::OpenCode,
            AppIcon::Pi,
            AppIcon::Plus,
            AppIcon::Question,
            AppIcon::SpinnerGap,
            AppIcon::Stop,
            AppIcon::Trash,
            AppIcon::UserFocus,
            AppIcon::WarningCircle,
            AppIcon::X,
            AppIcon::XCircle,
        ] {
            assert_themeable(icon.path().as_ref());
        }

        for icon in [
            IconName::CaseSensitive,
            IconName::Check,
            IconName::ChevronDown,
            IconName::ChevronLeft,
            IconName::ChevronRight,
            IconName::CircleCheck,
            IconName::CircleX,
            IconName::Close,
            IconName::ExternalLink,
            IconName::Eye,
            IconName::EyeOff,
            IconName::Inbox,
            IconName::Info,
            IconName::Loader,
            IconName::Plus,
            IconName::Replace,
            IconName::Search,
            IconName::TriangleAlert,
        ] {
            assert_themeable(icon.path().as_ref());
        }
    }

    fn assert_themeable(path: &str) {
        let bytes = AppAssets
            .load(path)
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
