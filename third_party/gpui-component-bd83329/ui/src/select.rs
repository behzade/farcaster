use gpui::{App, Hsla, IntoElement, RenderOnce, Styled as _, Window, prelude::FluentBuilder as _};

use crate::{Icon, IconName, Sizable as _, Size};

#[derive(IntoElement)]
pub struct Caret {
    size: Size,
    color: Option<Hsla>,
}

impl Caret {
    pub fn new(size: Size) -> Self {
        Self { size, color: None }
    }

    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Caret {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(IconName::ChevronDown)
            .with_size(match self.size {
                Size::XSmall => Size::XSmall,
                Size::Small => Size::Small,
                _ => Size::Medium,
            })
            .when_some(self.color, |this, color| this.text_color(color))
    }
}
