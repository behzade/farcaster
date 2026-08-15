use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

const ICON_ROOT: &str = "icons/phosphor";
const ICON_PATHS: [&str; 4] = [
    "icons/phosphor/archive.svg",
    "icons/phosphor/arrow-clockwise.svg",
    "icons/phosphor/plus.svg",
    "icons/phosphor/stop.svg",
];

pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/phosphor/archive.svg" => {
                Some(include_bytes!("../assets/phosphor-icons/archive.svg"))
            }
            "icons/phosphor/arrow-clockwise.svg" => Some(include_bytes!(
                "../assets/phosphor-icons/arrow-clockwise.svg"
            )),
            "icons/phosphor/plus.svg" => Some(include_bytes!("../assets/phosphor-icons/plus.svg")),
            "icons/phosphor/stop.svg" => Some(include_bytes!("../assets/phosphor-icons/stop.svg")),
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
    ArrowClockwise,
    Plus,
    Stop,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::Archive => "archive",
            Self::ArrowClockwise => "arrow-clockwise",
            Self::Plus => "plus",
            Self::Stop => "stop",
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
    fn phosphor_icons_are_embedded_and_themeable() {
        for icon in [
            AppIcon::Archive,
            AppIcon::ArrowClockwise,
            AppIcon::Plus,
            AppIcon::Stop,
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
