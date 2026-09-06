//! Cursor coordinates are rendered-row / grapheme indices, never UTF-8 offsets
//! into Markdown source. Geometry is disposable; anchors survive line wrapping.
use super::*;
use unicode_segmentation::UnicodeSegmentation as _;

mod motions;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KeyboardCommand {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBackward,
    WordEnd,
    BigWordForward,
    BigWordBackward,
    BigWordEnd,
    PreviousWordEnd(bool),
    LineStart,
    FirstNonblank,
    LineEnd,
    LastNonblank,
    ScreenUp,
    ScreenDown,
    ScreenStart,
    ScreenEnd,
    ScreenFirst,
    FirstLine(isize),
    Paragraph(bool),
    Sentence(bool),
    MatchBracket,
    Find {
        character: char,
        forward: bool,
        till: bool,
    },
    RepeatFind(bool),
    Viewport(u8),
    Align(u8),
    SearchStart(bool),
    SearchChar(char),
    SearchBackspace,
    SearchAccept,
    SearchCancel,
    SearchNext(bool),
    SearchWord(bool),
    SwapAnchor,
    Start,
    End,
    Page(f32),
    Visual(bool),
    Cancel,
    Yank,
    Copy,
}

impl KeyboardCommand {
    pub(crate) fn repeats(self) -> bool {
        !matches!(
            self,
            Self::Start | Self::End | Self::Visual(_) | Self::Cancel | Self::Yank | Self::Copy
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    row: usize,
    cell: usize,
}

struct Cell {
    text: String,
    bounds: Bounds<Pixels>,
}

#[derive(Default)]
struct TextRow {
    cells: Vec<Cell>,
}

impl TextRow {
    fn from_layouts(
        mut runs: Vec<(gpui::SharedString, gpui::TextLayout)>,
        origin: gpui::Point<Pixels>,
    ) -> Self {
        // Preserve each run's logical (including RTL) text order. Order sibling
        // runs by their layout, not their entity IDs or paint registration order.
        runs.sort_by(|a, b| {
            a.1.bounds()
                .top()
                .partial_cmp(&b.1.bounds().top())
                .unwrap()
                .then_with(|| {
                    a.1.bounds()
                        .left()
                        .partial_cmp(&b.1.bounds().left())
                        .unwrap()
                })
        });
        let mut cells: Vec<Cell> = Vec::new();
        for (text, layout) in runs {
            if !text.is_empty()
                && let Some(previous) = cells.last()
            {
                cells.push(Cell {
                    text: "\n".into(),
                    bounds: Bounds::new(
                        previous.bounds.top_right(),
                        gpui::size(px(2.0), previous.bounds.size.height),
                    ),
                });
            }
            for (offset, grapheme) in text.grapheme_indices(true) {
                let Some(start) = layout.position_for_index(offset) else {
                    continue;
                };
                let end = layout
                    .position_for_index(offset + grapheme.len())
                    .filter(|end| end.y == start.y);
                let width = end.map_or(px(2.0), |end| (end.x - start.x).abs().max(px(2.0)));
                let x = end.map_or(start.x, |end| start.x.min(end.x));
                cells.push(Cell {
                    text: grapheme.into(),
                    bounds: Bounds::new(
                        point(x, start.y) - origin,
                        gpui::size(width, layout.line_height().max(px(1.0))),
                    ),
                });
            }
        }
        Self { cells }
    }

    fn line_range(&self, cell: usize) -> Range<usize> {
        let y = self.cells[cell].bounds.top();
        let same_line = |other: &Cell| (other.bounds.top() - y).abs() < px(1.0);
        let start = self.cells[..cell]
            .iter()
            .rposition(|other| !same_line(other))
            .map_or(0, |i| i + 1);
        let end = self.cells[cell..]
            .iter()
            .position(|other| !same_line(other))
            .map_or(self.cells.len(), |i| cell + i);
        start..end
    }

    fn nearest_column(&self, line: Range<usize>, x: Pixels) -> usize {
        line.min_by(|a, b| {
            (self.cells[*a].bounds.left() - x)
                .abs()
                .partial_cmp(&(self.cells[*b].bounds.left() - x).abs())
                .unwrap()
        })
        .unwrap_or(0)
    }

    fn nearest_point(&self, point: gpui::Point<Pixels>) -> usize {
        let Some((index, _)) = self.cells.iter().enumerate().min_by(|(_, left), (_, right)| {
            (left.bounds.center().y - point.y)
                .abs()
                .partial_cmp(&(right.bounds.center().y - point.y).abs())
                .unwrap()
        }) else {
            return 0;
        };
        self.nearest_column(self.line_range(index), point.x)
    }
}

#[derive(Default)]
pub(super) struct Keyboard {
    pub active: bool,
    cursor: Option<Position>,
    anchor: Option<Position>,
    linewise: bool,
    preferred_x: Option<Pixels>,
    preferred_column: Option<usize>,
    alignment: Option<u8>,
    geometry_dirty: bool,
    recent_motion: bool,
    caret_timeout: Option<gpui::Task<()>>,
    pending: std::collections::VecDeque<KeyboardCommand>,
    last_find: Option<(char, bool, bool)>,
    search: String,
    search_forward: bool,
    search_whole_word: bool,
    search_draft: Option<(String, bool)>,
    pending_click: Option<gpui::Point<Pixels>>,
    cache: BTreeMap<usize, TextRow>,
}

impl Keyboard {
    pub(super) fn clear_geometry(&mut self) {
        self.cache.clear();
        self.geometry_dirty = true;
    }

    pub(super) fn retain_visible(&mut self, visible: Range<usize>) {
        self.cache.retain(|row, _| {
            visible.contains(row)
                || self.cursor.is_some_and(|pos| pos.row == *row)
                || self.anchor.is_some_and(|pos| pos.row == *row)
        });
    }

    pub(super) fn cancel_selection(&mut self) -> bool {
        let changed = self.anchor.take().is_some() || !self.pending.is_empty();
        self.pending.clear();
        self.search_draft = None;
        changed
    }

    pub(super) fn update_row(
        &mut self,
        index: usize,
        runs: Vec<(gpui::SharedString, gpui::TextLayout)>,
        origin: gpui::Point<Pixels>,
    ) {
        let row = TextRow::from_layouts(runs, origin);
        if self.cache.get(&index).is_some_and(|previous| {
            !previous
                .cells
                .iter()
                .map(|cell| &cell.text)
                .eq(row.cells.iter().map(|cell| &cell.text))
        }) {
            self.invalidate(index..index + 1, false);
        }
        self.cache.insert(index, row);
    }

    fn caret_visible(&self) -> bool {
        self.active && (self.recent_motion || self.has_selection())
    }

    pub(super) fn has_selection(&self) -> bool {
        self.anchor.is_some()
    }

    #[cfg(test)]
    pub(super) fn mode(&self) -> &'static str {
        match (self.anchor, self.linewise) {
            (Some(_), true) => "VISUAL LINE",
            (Some(_), false) => "VISUAL",
            _ => "NORMAL",
        }
    }

    pub(super) fn request_click(&mut self, point: gpui::Point<Pixels>) {
        self.active = true;
        self.pending_click = Some(point);
    }

    /// Action buttons call `prevent_default`. Those clicks must not move the
    /// caret, drop visual selection, or consume a pending motion.
    pub(super) fn apply_pointer_down(
        &mut self,
        inside: bool,
        default_prevented: bool,
        click: gpui::Point<Pixels>,
    ) -> bool {
        if default_prevented {
            return false;
        }
        self.cancel_selection();
        if inside {
            self.request_click(click);
            true
        } else {
            false
        }
    }

    pub(super) fn set_active(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            if !active {
                self.cancel_selection();
                self.pending_click = None;
                self.recent_motion = false;
                self.caret_timeout = None;
            }
        }
    }

    fn place_at(
        &mut self,
        click: gpui::Point<Pixels>,
        row: usize,
        row_top_in_viewport: Pixels,
        load: &mut impl FnMut(usize) -> TextRow,
    ) {
        let text = self.row(row, load);
        if text.cells.is_empty() {
            return;
        }
        let cell = text.nearest_point(point(click.x, click.y - row_top_in_viewport));
        self.cursor = Some(Position { row, cell });
        self.anchor = None;
        self.linewise = false;
        self.preferred_x = None;
        self.preferred_column = None;
        self.recent_motion = true;
        self.pending.clear();
        self.search_draft = None;
    }

    pub(super) fn invalidate(&mut self, range: Range<usize>, structural: bool) {
        self.cache
            .retain(|row, _| !range.contains(row) && (!structural || *row < range.start));
        let affected =
            |pos: Position| range.contains(&pos.row) || (structural && pos.row >= range.start);
        if let (Some(anchor), Some(cursor)) = (self.anchor, self.cursor)
            && range.start <= anchor.row.max(cursor.row)
            && range.end > anchor.row.min(cursor.row)
        {
            self.anchor = None;
        }
        if self.cursor.is_some_and(affected) || self.anchor.is_some_and(affected) {
            self.cursor = None;
            self.anchor = None;
            self.preferred_x = None;
            self.preferred_column = None;
            self.geometry_dirty = true;
        }
    }

    fn row(&mut self, index: usize, load: &mut impl FnMut(usize) -> TextRow) -> &TextRow {
        self.cache.entry(index).or_insert_with(|| load(index))
    }

    fn boundary(
        &mut self,
        from: usize,
        forward: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Option<Position> {
        let mut row = from;
        while row < count {
            let len = self.row(row, load).cells.len();
            if len > 0 {
                return Some(Position {
                    row,
                    cell: if forward { 0 } else { len - 1 },
                });
            }
            row = if forward {
                row + 1
            } else {
                row.checked_sub(1)?
            };
        }
        None
    }

    fn adjacent(
        &mut self,
        pos: Position,
        forward: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Option<Position> {
        if forward {
            if pos.cell + 1 < self.row(pos.row, load).cells.len() {
                return Some(Position {
                    cell: pos.cell + 1,
                    ..pos
                });
            }
            self.boundary(pos.row + 1, true, count, load)
        } else if pos.cell > 0 {
            Some(Position {
                cell: pos.cell - 1,
                ..pos
            })
        } else {
            self.boundary(pos.row.checked_sub(1)?, false, count, load)
        }
    }

    fn vertical(
        &mut self,
        pos: Position,
        forward: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        self.vertical_line(pos, forward, false, count, load)
    }

    fn vertical_line(
        &mut self,
        pos: Position,
        forward: bool,
        display: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        let x = *self
            .preferred_x
            .get_or_insert(self.cache[&pos.row].cells[pos.cell].bounds.left());
        let row = self.row(pos.row, load);
        let line = row.motion_line(pos.cell, display);
        let column = pos.cell.saturating_sub(line.start);
        let next = if forward {
            if !display
                && row
                    .cells
                    .get(line.end)
                    .is_some_and(|cell| cell.text == "\n")
            {
                line.end + 1
            } else {
                line.end
            }
        } else {
            line.start.saturating_sub(1)
        };
        let target = if (forward && next < row.cells.len()) || (!forward && line.start > 0) {
            Some(Position { cell: next, ..pos })
        } else if forward {
            self.boundary(pos.row + 1, true, count, load)
        } else {
            pos.row
                .checked_sub(1)
                .and_then(|row| self.boundary(row, false, count, load))
        };
        let Some(target) = target else { return pos };
        let column = *self.preferred_column.get_or_insert(column);
        let row = self.row(target.row, load);
        let line = row.motion_line(target.cell, display);
        Position {
            cell: if display {
                row.nearest_column(line, x)
            } else {
                (line.start + column).min(line.end - 1)
            },
            ..target
        }
    }

    fn word(
        &mut self,
        pos: Position,
        forward: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        self.word_motion(pos, forward, false, false, count, load)
    }

    /// Resolve queued commands against lazily measured rows. Clipboard and
    /// viewport effects are applied by the list after releasing the loader.
    fn apply_pending(
        &mut self,
        count: usize,
        viewport_height: Pixels,
        mut resume_tail: bool,
        viewport: [ListOffset; 3],
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> (Option<String>, bool, bool) {
        let mut copied = None;
        let mut moved = false;
        while let Some(command) = self.pending.pop_front() {
            let Some(pos) = self.cursor else { break };
            use KeyboardCommand::*;
            let next = match command {
                Left | Right => {
                    let line = self.row(pos.row, load).motion_line(pos.cell, false);
                    Position {
                        cell: if command == Right {
                            (pos.cell + 1).min(line.end - 1)
                        } else {
                            pos.cell.saturating_sub(1).max(line.start)
                        },
                        ..pos
                    }
                }
                Up | Down => self.vertical(pos, command == Down, count, load),
                WordForward | WordBackward => self.word(pos, command == WordForward, count, load),
                ScreenUp | ScreenDown => {
                    self.vertical_line(pos, command == ScreenDown, true, count, load)
                }
                Viewport(which) => self
                    .viewport_position(viewport[which as usize], count, load)
                    .unwrap_or(pos),
                Align(which) => {
                    self.alignment = Some(which);
                    pos
                }
                SwapAnchor => {
                    if let Some(anchor) = self.anchor.replace(pos) {
                        anchor
                    } else {
                        self.anchor = None;
                        pos
                    }
                }
                SearchStart(forward) => {
                    self.search_draft = Some((String::new(), forward));
                    pos
                }
                SearchChar(character) => {
                    if let Some((text, _)) = &mut self.search_draft {
                        text.push(character);
                    }
                    pos
                }
                SearchBackspace => {
                    if let Some((text, _)) = &mut self.search_draft {
                        if let Some((index, _)) = text.grapheme_indices(true).last() {
                            text.truncate(index);
                        }
                    }
                    pos
                }
                SearchCancel => {
                    self.search_draft = None;
                    pos
                }
                SearchAccept => {
                    if let Some((text, forward)) = self.search_draft.take() {
                        if !text.is_empty() {
                            self.search = text;
                            self.search_whole_word = false;
                        }
                        self.search_forward = forward;
                    }
                    self.search_motion(pos, self.search_forward, count, load)
                }
                SearchNext(reverse) => {
                    self.search_motion(pos, self.search_forward != reverse, count, load)
                }
                SearchWord(forward) => {
                    self.search = self.word_at(pos, load);
                    self.search_whole_word = true;
                    self.search_forward = forward;
                    self.search_motion(pos, forward, count, load)
                }
                Start => self.boundary(0, true, count, load).unwrap_or(pos),
                End => self.boundary(count - 1, false, count, load).unwrap_or(pos),
                Page(pages) => {
                    let steps = (f32::from(viewport_height) * pages.abs()
                        / f32::from(self.cache[&pos.row].cells[pos.cell].bounds.size.height))
                    .max(1.0) as usize;
                    let mut next = pos;
                    for _ in 0..steps {
                        next = self.vertical_line(next, pages > 0.0, true, count, load);
                    }
                    next
                }
                Visual(linewise) => {
                    self.anchor = if self.anchor.is_some() && self.linewise == linewise {
                        None
                    } else {
                        Some(self.anchor.unwrap_or(pos))
                    };
                    self.linewise = linewise;
                    pos
                }
                Cancel => {
                    self.anchor = None;
                    pos
                }
                Yank | Copy => {
                    copied = self.copy(load);
                    if command == Yank {
                        self.anchor = None;
                    }
                    pos
                }
                motion => self.motion(pos, motion, count, load),
            };
            moved |= next != pos;
            self.cursor = Some(next);
            resume_tail = command == End && self.anchor.is_none();
            if !matches!(command, Up | Down | ScreenUp | ScreenDown | Page(_)) {
                self.preferred_x = None;
                self.preferred_column = None;
            }
        }
        (copied, resume_tail, moved)
    }

    fn selected_range(&self, row: usize) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let cursor = self.cursor?;
        let start = anchor.min(cursor);
        let end = anchor.max(cursor);
        if row < start.row || row > end.row {
            return None;
        }
        let text = self.cache.get(&row)?;
        if text.cells.is_empty() {
            return None;
        }
        let from = if row == start.row { start.cell } else { 0 };
        let to = if row == end.row {
            end.cell + 1
        } else {
            text.cells.len()
        };
        Some(if self.linewise {
            text.motion_line(from, false).start..text.motion_line(to - 1, false).end
        } else {
            from..to
        })
    }

    fn copy(&mut self, load: &mut impl FnMut(usize) -> TextRow) -> Option<String> {
        let (anchor, cursor) = (self.anchor?, self.cursor?);
        let mut text = String::new();
        for row in anchor.row.min(cursor.row)..=anchor.row.max(cursor.row) {
            self.row(row, load);
            if let Some(range) = self.selected_range(row) {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                for cell in &self.cache[&row].cells[range] {
                    text.push_str(&cell.text);
                }
            }
            if row != anchor.row && row != cursor.row {
                self.cache.remove(&row);
            }
        }
        if self.linewise && !text.ends_with('\n') {
            text.push('\n');
        }
        Some(text)
    }
}

impl TranscriptListState {
    pub(crate) fn watch_keyboard_mode(&self, view: EntityId) {
        self.0.borrow_mut().keyboard_observer = Some(view);
    }

    pub(crate) fn has_keyboard_selection(&self) -> bool {
        self.0.borrow().keyboard.has_selection()
    }

    #[cfg(test)]
    pub(crate) fn keyboard_mode(&self) -> &'static str {
        self.0.borrow().keyboard.mode()
    }

    pub(crate) fn set_keyboard_active(&self, active: bool) {
        self.0.borrow_mut().keyboard.set_active(active);
    }

    pub(crate) fn keyboard_command(&self, command: KeyboardCommand) {
        let mut state = self.0.borrow_mut();
        state.clear_selection();
        state.keyboard.active = true;
        state.keyboard.pending.push_back(command);
    }
}

impl TranscriptList {
    fn keyboard_row(
        &mut self,
        index: usize,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> TextRow {
        let mut row = (self.render_row)(index, window, cx);
        let height = row
            .layout_as_root(
                gpui::size(bounds.size.width.into(), AvailableSpace::MinContent),
                window,
                cx,
            )
            .height
            .max(px(1.0));
        self.state.0.borrow_mut().heights.set_height(index, height);
        // Measuring a virtualized destination must not install offscreen focus
        // targets, hitboxes, tooltips, or deferred draws into the real frame.
        let result: Result<(), TextRow> = window.transact(|window| {
            let (_, runs) =
                gpui::TextLayout::capture(cx, |cx| row.prepaint_at(bounds.origin, window, cx));
            Err(TextRow::from_layouts(runs, bounds.origin))
        });
        result.err().unwrap()
    }

    pub(super) fn prepare_keyboard(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let (mut keyboard, count, scroll_top, following, viewport, click_row, click_top) = {
            let mut state = self.state.0.borrow_mut();
            if (!state.keyboard.active && state.keyboard.pending_click.is_none())
                || state.heights.is_empty()
            {
                return false;
            }
            if state.keyboard.pending_click.is_some() {
                state.keyboard.active = true;
            }
            let pending_click = state.keyboard.pending_click;
            let click_row = pending_click.map(|click| {
                state
                    .heights
                    .row_at((state.scroll_y + click.y).max(px(0.0)))
                    .min(state.heights.len() - 1)
            });
            let click_top = click_row.map(|row| state.heights.prefix(row) - state.scroll_y);
            (
                std::mem::take(&mut state.keyboard),
                state.heights.len(),
                state.logical_scroll_top(),
                state.following_tail,
                [0.0, 0.5, 1.0].map(|fraction| {
                    let y = state.scroll_y + (bounds.size.height - px(1.0)) * fraction;
                    let item_ix = state.heights.row_at(y).min(state.heights.len() - 1);
                    ListOffset {
                        item_ix,
                        offset_in_item: y - state.heights.prefix(item_ix),
                    }
                }),
                click_row,
                click_top,
            )
        };
        let top = scroll_top.item_ix.min(count - 1);
        let has_commands = !keyboard.pending.is_empty();
        let placed_click = keyboard.pending_click.take().zip(click_row).zip(click_top);
        let click_placed = placed_click.is_some();
        let changed =
            std::mem::take(&mut keyboard.geometry_dirty) || has_commands || click_placed;
        let initializing = keyboard.cursor.is_none() && placed_click.is_none();
        let resume_tail = !has_commands && following && placed_click.is_none();
        let (copied, resume_tail, moved) = {
            let mut load = |index| self.keyboard_row(index, bounds, window, cx);
            if let Some(((click, row), row_top)) = placed_click {
                keyboard.place_at(click, row, row_top, &mut load);
            }
            if let Some(pos) = keyboard.cursor {
                let len = keyboard.row(pos.row.min(count - 1), &mut load).cells.len();
                keyboard.cursor = (len > 0).then_some(Position {
                    row: pos.row.min(count - 1),
                    cell: pos.cell.min(len.saturating_sub(1)),
                });
            }
            if keyboard.cursor.is_none() {
                keyboard.cursor = keyboard
                    .boundary(top, true, count, &mut load)
                    .or_else(|| keyboard.boundary(top, false, count, &mut load));
            }
            if resume_tail {
                keyboard.cursor = keyboard.boundary(count - 1, false, count, &mut load);
            }
            if initializing
                && !resume_tail
                && let Some(pos) = keyboard.cursor
            {
                let offset = if pos.row == top {
                    scroll_top.offset_in_item
                } else {
                    px(0.0)
                };
                let cells = &keyboard.cache[&pos.row].cells;
                keyboard.cursor = Some(Position {
                    cell: cells
                        .iter()
                        .position(|cell| cell.bounds.bottom() > offset)
                        .unwrap_or(cells.len() - 1),
                    ..pos
                });
            }
            let pending = keyboard.apply_pending(
                count,
                bounds.size.height,
                resume_tail,
                viewport,
                &mut load,
            );
            (
                pending.0,
                pending.1,
                pending.2 || click_placed,
            )
        };
        if let Some(text) = copied {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        if moved {
            keyboard.recent_motion = true;
            let state = Rc::downgrade(&self.state.0);
            let timer = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(2));
            keyboard.caret_timeout = Some(cx.spawn(async move |cx| {
                timer.await;
                let _ = cx.update(|cx| {
                    let Some(state) = state.upgrade() else { return };
                    let mut state = state.borrow_mut();
                    state.keyboard.recent_motion = false;
                    if let Some(view) = state.keyboard_observer {
                        cx.notify(view);
                    }
                });
            }));
        }
        let mut state = self.state.0.borrow_mut();
        if changed {
            state.pending_scroll = px(0.0);
            state.following_tail = resume_tail;
            if resume_tail {
                state.scroll_y = state.maximum_scroll();
            } else if let Some(pos) = keyboard.cursor {
                let cell = &keyboard.cache[&pos.row].cells[pos.cell];
                let top = state.heights.prefix(pos.row) + cell.bounds.top();
                let bottom = top + cell.bounds.size.height;
                if top < state.scroll_y {
                    state.scroll_y = top;
                } else if bottom > state.scroll_y + bounds.size.height {
                    state.scroll_y = bottom - bounds.size.height;
                }
                state.clamp_scroll();
            }
        }
        if let Some(alignment) = keyboard.alignment.take()
            && let Some(pos) = keyboard.cursor
        {
            let cell = &keyboard.cache[&pos.row].cells[pos.cell];
            let offset = (bounds.size.height - cell.bounds.size.height) * (alignment as f32 / 2.0);
            state.scroll_y = state.heights.prefix(pos.row) + cell.bounds.top() - offset;
            state.clamp_scroll();
        }
        state.keyboard = keyboard;
        if changed && let Some(view) = state.keyboard_observer {
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
        changed
    }

    pub(super) fn paint_keyboard(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let state = self.state.0.borrow();
        let keyboard = &state.keyboard;
        if !keyboard.active {
            return;
        }
        let theme = crate::app::ui::theme::THEME;
        for row in state.layout_range(px(0.0)) {
            let Some(text) = keyboard.cache.get(&row) else {
                continue;
            };
            let origin = bounds.origin + point(px(0.0), state.heights.prefix(row) - state.scroll_y);
            if let Some(range) = keyboard.selected_range(row) {
                for cell in &text.cells[range] {
                    window.paint_quad(gpui::fill(
                        Bounds::new(origin + cell.bounds.origin, cell.bounds.size),
                        theme.colors.text_selection.opacity(0.45),
                    ));
                }
            }
            if keyboard.caret_visible()
                && let Some(cursor) = keyboard.cursor.filter(|cursor| cursor.row == row)
                && let Some(cell) = text.cells.get(cursor.cell)
            {
                window.paint_quad(gpui::fill(
                    Bounds::new(
                        origin + cell.bounds.origin,
                        gpui::size(px(2.0), cell.bounds.size.height),
                    ),
                    theme.colors.accent,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
