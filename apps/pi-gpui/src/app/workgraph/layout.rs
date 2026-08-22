use gpui::Pixels;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoardLayoutMode {
    Wide,
    Compact,
    Narrow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IssueDetailShell {
    Embedded,
    Sheet,
}

impl IssueDetailShell {
    #[must_use]
    pub(crate) const fn shows_sheet(self, review_expanded: bool) -> bool {
        !review_expanded && matches!(self, Self::Sheet)
    }
}

pub(crate) const WIDE_BOARD_MIN_WIDTH: f32 = 1_180.0;
pub(crate) const COMPACT_BOARD_MIN_WIDTH: f32 = 960.0;
pub(crate) const DETAIL_WIDTH: f32 = 400.0;
pub(crate) const DETAIL_MIN_WIDTH: f32 = 360.0;

#[must_use]
pub(crate) fn board_layout_mode(width: Pixels) -> BoardLayoutMode {
    let width = f32::from(width);
    if width >= WIDE_BOARD_MIN_WIDTH {
        BoardLayoutMode::Wide
    } else if width >= COMPACT_BOARD_MIN_WIDTH {
        BoardLayoutMode::Compact
    } else {
        BoardLayoutMode::Narrow
    }
}

#[must_use]
pub(crate) const fn issue_detail_shell(layout: BoardLayoutMode) -> IssueDetailShell {
    match layout {
        BoardLayoutMode::Wide | BoardLayoutMode::Compact => IssueDetailShell::Embedded,
        BoardLayoutMode::Narrow => IssueDetailShell::Sheet,
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{BoardLayoutMode, IssueDetailShell, board_layout_mode, issue_detail_shell};

    #[test]
    fn layout_thresholds_use_the_narrow_sheet_only_below_960() {
        assert_eq!(board_layout_mode(px(959.0)), BoardLayoutMode::Narrow);
        assert_eq!(board_layout_mode(px(960.0)), BoardLayoutMode::Compact);
        assert_eq!(board_layout_mode(px(1_114.0)), BoardLayoutMode::Compact);
        assert_eq!(board_layout_mode(px(1_179.0)), BoardLayoutMode::Compact);
        assert_eq!(board_layout_mode(px(1_180.0)), BoardLayoutMode::Wide);
    }

    #[test]
    fn detail_shell_tracks_the_board_layout_boundary() {
        assert_eq!(
            issue_detail_shell(board_layout_mode(px(959.0))),
            IssueDetailShell::Sheet
        );
        assert_eq!(
            issue_detail_shell(board_layout_mode(px(960.0))),
            IssueDetailShell::Embedded
        );
        assert_eq!(
            issue_detail_shell(BoardLayoutMode::Wide),
            IssueDetailShell::Embedded
        );
        assert!(IssueDetailShell::Sheet.shows_sheet(false));
        assert!(!IssueDetailShell::Sheet.shows_sheet(true));
    }
}
