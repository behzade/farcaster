use crate::app::ui::theme::theme;

pub(super) fn dimensions(
    viewport_width: f32,
    viewport_height: f32,
    models: usize,
) -> (f32, f32, f32) {
    let gutter = f32::from(theme().size(32.0));
    let width = (viewport_width - gutter).clamp(0.0, f32::from(theme().size(380.0)));
    let height = (viewport_height - gutter).clamp(0.0, f32::from(theme().size(480.0)));
    let results = (models as f32 * gutter).min((height - f32::from(theme().size(150.0))).max(0.0));
    (width, height, results)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
