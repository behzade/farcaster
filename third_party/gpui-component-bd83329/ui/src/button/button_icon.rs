use crate::{Icon, Sizable, Size, spinner::Spinner};
use gpui::{App, IntoElement, RenderOnce, Window, prelude::FluentBuilder};

/// Button icon which can be an Icon, Spinner, or Progress use for `icon` method of Button.
#[doc(hidden)]
#[derive(IntoElement)]
pub struct ButtonIcon {
    icon: ButtonIconVariant,
    loading_icon: Option<Icon>,
    loading: bool,
    size: Size,
}

impl<T> From<T> for ButtonIcon
where
    T: Into<ButtonIconVariant>,
{
    fn from(icon: T) -> Self {
        ButtonIcon::new(icon)
    }
}

impl ButtonIcon {
    /// Creates a new ButtonIcon with the given icon.
    pub fn new(icon: impl Into<ButtonIconVariant>) -> Self {
        Self {
            icon: icon.into(),
            loading_icon: None,
            loading: false,
            size: Size::Medium,
        }
    }

    pub(crate) fn loading_icon(mut self, icon: Option<Icon>) -> Self {
        self.loading_icon = icon;
        self
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
}

impl Sizable for ButtonIcon {
    fn with_size(mut self, size: impl Into<crate::Size>) -> Self {
        self.size = size.into();
        self
    }
}

/// Button icon used by the extracted Button implementation.
#[doc(hidden)]
#[derive(IntoElement)]
pub enum ButtonIconVariant {
    Icon(Icon),
    Spinner(Spinner),
}

impl<T> From<T> for ButtonIconVariant
where
    T: Into<Icon>,
{
    fn from(icon: T) -> Self {
        Self::Icon(icon.into())
    }
}

impl From<Spinner> for ButtonIconVariant {
    fn from(spinner: Spinner) -> Self {
        Self::Spinner(spinner)
    }
}

impl ButtonIconVariant {
    /// Returns true if the ButtonIconKind is an Icon.
    #[inline]
    pub(crate) fn is_spinner(&self) -> bool {
        matches!(self, Self::Spinner(_))
    }
}

impl Sizable for ButtonIconVariant {
    fn with_size(self, size: impl Into<crate::Size>) -> Self {
        match self {
            Self::Icon(icon) => Self::Icon(icon.with_size(size)),
            Self::Spinner(spinner) => Self::Spinner(spinner.with_size(size)),
        }
    }
}

impl RenderOnce for ButtonIconVariant {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Icon(icon) => icon.into_any_element(),
            Self::Spinner(spinner) => spinner.into_any_element(),
        }
    }
}

impl RenderOnce for ButtonIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        if self.loading {
            if self.icon.is_spinner() {
                self.icon.with_size(self.size).into_any_element()
            } else {
                Spinner::new()
                    .when_some(self.loading_icon, |this, icon| this.icon(icon))
                    .with_size(self.size)
                    .into_any_element()
            }
        } else {
            self.icon.with_size(self.size).into_any_element()
        }
    }
}


#[cfg(test)]
#[path = "button_icon_tests.rs"]
mod tests;
