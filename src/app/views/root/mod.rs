mod actions;
mod draft;
mod keybindings;
mod lifecycle;
mod overlays;
mod shell;

use gpui::{InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, div};

use super::FarcasterApp;
use crate::app::ui::{
    layout::layout_mode,
    theme::{THEME, ui_font},
};
use crate::app::{APP_INPUT_CONTEXT, AppSurface, NATIVE_INPUT_CONTEXT};

impl Render for FarcasterApp {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let _timing = crate::app::infrastructure::performance::Timing::new("render.root");
        self.prepare_root_render(window, cx);

        let mode = layout_mode(window.viewport_size().width);
        let entity = cx.entity().downgrade();
        let key_context = match self.surface {
            AppSurface::Chat | AppSurface::Work => APP_INPUT_CONTEXT,
            AppSurface::Editor | AppSurface::Terminal => NATIVE_INPUT_CONTEXT,
        };
        let work_active = self.surface == AppSurface::Work;
        let main = self.render_workspace_main(entity.clone(), mode);
        let session_rail_width = self.session_rail_view.read(cx).width();
        let run_panel_width = self.run_panel_view.read(cx).width();
        let shell = self.render_inline_shell(
            entity.clone(),
            mode,
            main,
            session_rail_width,
            run_panel_width,
        );
        let picker = self.render_picker(entity.clone(), cx);
        let root = div()
            .relative()
            .size_full()
            .bg(THEME.colors.canvas)
            .font(ui_font())
            .key_context(key_context)
            .text_color(THEME.colors.text)
            .text_size(THEME.type_scale.body);
        let root = actions::bind(root, cx).child(shell);

        self.render_root_overlays(root, entity, picker, work_active, cx)
    }
}
