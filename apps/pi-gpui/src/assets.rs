use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString};
use gpui_component::IconNamed;

const ICON_ROOT: &str = "icons/phosphor";
const ICON_PATHS: [&str; 7] = [
    "icons/phosphor/chat-circle.svg",
    "icons/phosphor/check-circle.svg",
    "icons/phosphor/folder.svg",
    "icons/phosphor/folder-plus.svg",
    "icons/phosphor/magnifying-glass.svg",
    "icons/phosphor/plus.svg",
    "icons/phosphor/x.svg",
];

pub(crate) struct AppAssets;

impl AppAssets {
    pub(crate) fn load_fonts(&self, cx: &App) -> Result<()> {
        cx.text_system().add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Bold.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-Italic.ttf")),
            Cow::Borrowed(include_bytes!("../assets/lilex/Lilex-BoldItalic.ttf")),
        ])
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/phosphor/chat-circle.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/chat-circle.svg"))
            }
            "icons/phosphor/check-circle.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/check-circle.svg"))
            }
            "icons/phosphor/folder.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/folder.svg"))
            }
            "icons/phosphor/folder-plus.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/folder-plus.svg"))
            }
            "icons/phosphor/magnifying-glass.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/magnifying-glass.svg"
            )),
            "icons/phosphor/plus.svg" => Some(include_bytes!("../assets/phosphor-icons/plus.svg")),
            "icons/phosphor/x.svg" => Some(include_bytes!("../assets/phosphor-icons/x.svg")),
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
    ChatCircle,
    CheckCircle,
    Folder,
    FolderPlus,
    MagnifyingGlass,
    Plus,
    X,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::ChatCircle => "chat-circle",
            Self::CheckCircle => "check-circle",
            Self::Folder => "folder",
            Self::FolderPlus => "folder-plus",
            Self::MagnifyingGlass => "magnifying-glass",
            Self::Plus => "plus",
            Self::X => "x",
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
    fn bundled_lilex_fonts_have_true_type_headers() {
        for bytes in [
            include_bytes!("../assets/lilex/Lilex-Regular.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-Bold.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-Italic.ttf").as_slice(),
            include_bytes!("../assets/lilex/Lilex-BoldItalic.ttf").as_slice(),
        ] {
            assert!(matches!(&bytes[..4], b"\0\x01\0\0" | b"OTTO"));
        }
    }

    #[test]
    fn phosphor_icons_are_embedded_and_themeable() {
        for icon in [
            AppIcon::ChatCircle,
            AppIcon::CheckCircle,
            AppIcon::Folder,
            AppIcon::FolderPlus,
            AppIcon::MagnifyingGlass,
            AppIcon::Plus,
            AppIcon::X,
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
