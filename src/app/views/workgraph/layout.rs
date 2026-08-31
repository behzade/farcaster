#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoardLayoutMode {
    Wide,
    Narrow,
}

pub(crate) const DETAIL_WIDTH: f32 = 400.0;
pub(crate) const DETAIL_MIN_WIDTH: f32 = 360.0;
