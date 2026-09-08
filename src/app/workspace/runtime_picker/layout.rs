pub(super) fn dimensions(
    viewport_width: f32,
    viewport_height: f32,
    models: usize,
) -> (f32, f32, f32) {
    let width = (viewport_width - 32.0).clamp(0.0, 380.0);
    let height = (viewport_height - 32.0).clamp(0.0, 480.0);
    let results = (models as f32 * 32.0).min((height - 150.0).max(0.0));
    (width, height, results)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
