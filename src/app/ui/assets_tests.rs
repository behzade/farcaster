use gpui::AssetSource as _;
use gpui_component::{IconName, IconNamed as _};

use super::{AppAssets, AppIcon};

#[test]
fn bundled_fonts_have_true_type_headers() {
    for bytes in [
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-Regular.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-Italic.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-Medium.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-MediumItalic.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-SemiBold.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-Bold.ttf").as_slice(),
        include_bytes!("../../../assets/ibm-plex-sans/IBMPlexSans-BoldItalic.ttf").as_slice(),
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
    assert_eq!(AppIcon::for_harness("cursor-cli"), AppIcon::Cursor);
    assert_eq!(AppIcon::for_harness("opencode2"), AppIcon::OpenCode);
    assert_eq!(AppIcon::for_harness("claude"), AppIcon::Claude);
    assert_eq!(
        AppIcon::for_harness("antigravity-acp"),
        AppIcon::Antigravity
    );
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
        AppIcon::Antigravity,
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
        AppIcon::Claude,
        AppIcon::Code,
        AppIcon::Codex,
        AppIcon::Copy,
        AppIcon::Cursor,
        AppIcon::Eye,
        AppIcon::Folder,
        AppIcon::FolderPlus,
        AppIcon::Ghostty,
        AppIcon::GitFork,
        AppIcon::Hammer,
        AppIcon::Hourglass,
        AppIcon::Key,
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
