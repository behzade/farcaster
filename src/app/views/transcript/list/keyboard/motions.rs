use super::*;

impl Cell {
    fn class(&self, big: bool) -> u8 {
        if self.text.chars().all(char::is_whitespace) {
            0
        } else if big || self.text.chars().any(|c| c.is_alphanumeric() || c == '_') {
            1
        } else {
            2
        }
    }
}

impl TextRow {
    pub(super) fn motion_line(&self, cell: usize, display: bool) -> Range<usize> {
        if display {
            return self.line_range(cell);
        }
        let start = self.cells[..cell]
            .iter()
            .rposition(|c| c.text == "\n")
            .map_or(0, |i| i + 1);
        let end = self.cells[cell..]
            .iter()
            .position(|c| c.text == "\n")
            .map_or(self.cells.len(), |i| cell + i);
        start..end.max(start + 1)
    }

    fn line_cell(&self, pos: usize, command: KeyboardCommand) -> usize {
        use KeyboardCommand::*;
        let line = self.motion_line(
            pos,
            matches!(command, ScreenStart | ScreenFirst | ScreenEnd),
        );
        match command {
            LineEnd | ScreenEnd => line.end - 1,
            LastNonblank => line
                .clone()
                .rfind(|i| self.cells[*i].class(false) != 0)
                .unwrap_or(line.start),
            FirstNonblank | ScreenFirst => line
                .clone()
                .find(|i| self.cells[*i].class(false) != 0)
                .unwrap_or(line.start),
            _ => line.start,
        }
    }
}

impl Keyboard {
    pub(super) fn word_motion(
        &mut self,
        pos: Position,
        forward: bool,
        end: bool,
        big: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        let mut current = pos;
        // b/e/ge first leave the current boundary; w starts by leaving its class.
        if forward && !end {
            let class = self.row(current.row, load).cells[current.cell].class(big);
            while let Some(next) = self.adjacent(current, true, count, load) {
                if next.row != current.row
                    || self.row(next.row, load).cells[next.cell].class(big) != class
                {
                    current = next;
                    break;
                }
                current = next;
            }
        } else if let Some(next) = self.adjacent(current, forward, count, load) {
            current = next;
            if !forward && end {
                // ge skips the rest of the current word before seeking the previous end.
                let class = self.row(pos.row, load).cells[pos.cell].class(big);
                while current.row == pos.row
                    && class != 0
                    && self.row(current.row, load).cells[current.cell].class(big) == class
                {
                    let Some(next) = self.adjacent(current, false, count, load) else {
                        return current;
                    };
                    current = next;
                }
            }
        } else {
            return pos;
        }
        while self.row(current.row, load).cells[current.cell].class(big) == 0 {
            let Some(next) = self.adjacent(current, forward, count, load) else {
                return current;
            };
            current = next;
        }
        if forward == end {
            let class = self.row(current.row, load).cells[current.cell].class(big);
            while let Some(next) = self.adjacent(current, forward, count, load) {
                if next.row != current.row
                    || self.row(next.row, load).cells[next.cell].class(big) != class
                {
                    break;
                }
                current = next;
            }
        }
        current
    }

    pub(super) fn motion(
        &mut self,
        pos: Position,
        command: KeyboardCommand,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        use KeyboardCommand::*;
        match command {
            WordEnd => self.word_motion(pos, true, true, false, count, load),
            BigWordForward => self.word_motion(pos, true, false, true, count, load),
            BigWordBackward => self.word_motion(pos, false, false, true, count, load),
            BigWordEnd => self.word_motion(pos, true, true, true, count, load),
            PreviousWordEnd(big) => self.word_motion(pos, false, true, big, count, load),
            LineStart | LineEnd | FirstNonblank | LastNonblank | ScreenStart | ScreenEnd
            | ScreenFirst => Position {
                cell: self.row(pos.row, load).line_cell(pos.cell, command),
                ..pos
            },
            FirstLine(direction) => {
                let target = self.vertical(pos, direction > 0, count, load);
                Position {
                    cell: self
                        .row(target.row, load)
                        .line_cell(target.cell, FirstNonblank),
                    ..target
                }
            }
            Find {
                character,
                forward,
                till,
            } => {
                self.last_find = Some((character, forward, till));
                self.find(pos, character, forward, till, false, load)
            }
            RepeatFind(reverse) => self
                .last_find
                .map(|(c, forward, till)| self.find(pos, c, forward != reverse, till, true, load))
                .unwrap_or(pos),
            Paragraph(forward) | Sentence(forward) => {
                self.text_boundary(pos, forward, matches!(command, Sentence(_)), count, load)
            }
            MatchBracket => self.match_bracket(pos, count, load),
            _ => pos,
        }
    }

    fn find(
        &mut self,
        pos: Position,
        character: char,
        forward: bool,
        till: bool,
        repeated: bool,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        let row = self.row(pos.row, load);
        let line = row.motion_line(pos.cell, false);
        let mut cell = pos.cell;
        // Repeating t/T must not find the same adjacent character again.
        let skip = if till && repeated { 2 } else { 1 };
        for _ in 0..skip {
            let Some(next) = (if forward {
                cell.checked_add(1)
            } else {
                cell.checked_sub(1)
            }) else {
                return pos;
            };
            cell = next;
        }
        while line.contains(&cell) {
            if row.cells[cell].text.starts_with(character) {
                return Position {
                    cell: if till {
                        if forward { cell - 1 } else { cell + 1 }
                    } else {
                        cell
                    },
                    ..pos
                };
            }
            let Some(next) = (if forward {
                cell.checked_add(1)
            } else {
                cell.checked_sub(1)
            }) else {
                break;
            };
            cell = next;
        }
        pos
    }

    fn text_boundary(
        &mut self,
        pos: Position,
        forward: bool,
        sentence: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        let mut current = pos;
        while let Some(next) = self.adjacent(current, forward, count, load) {
            let candidate = next;
            let row_start = candidate.cell == 0;
            let is_boundary = if row_start {
                true
            } else {
                let row = self.row(candidate.row, load);
                if sentence {
                    row.cells[candidate.cell].class(false) != 0 && {
                        let mut before = candidate.cell;
                        while before > 0 && row.cells[before - 1].class(false) == 0 {
                            before -= 1;
                        }
                        before < candidate.cell
                            && before > 0
                            && matches!(row.cells[before - 1].text.as_str(), "." | "!" | "?")
                    }
                } else {
                    row.cells[candidate.cell].text == "\n"
                        && (candidate.cell == 0 || row.cells[candidate.cell - 1].text == "\n")
                }
            };
            if is_boundary {
                return candidate;
            }
            current = next;
        }
        current
    }

    fn match_bracket(
        &mut self,
        pos: Position,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        let row = self.row(pos.row, load);
        let line = row.motion_line(pos.cell, false);
        let Some(cell) = (pos.cell..line.end).find(|i| {
            matches!(
                row.cells[*i].text.as_str(),
                "(" | ")" | "[" | "]" | "{" | "}"
            )
        }) else {
            return pos;
        };
        let (open, close, forward) = match row.cells[cell].text.as_str() {
            "(" => ("(", ")", true),
            ")" => ("(", ")", false),
            "[" => ("[", "]", true),
            "]" => ("[", "]", false),
            "{" => ("{", "}", true),
            _ => ("{", "}", false),
        };
        let mut current = Position { cell, ..pos };
        let mut depth = 1;
        while let Some(next) = self.adjacent(current, forward, count, load) {
            let text = self.row(next.row, load).cells[next.cell].text.as_str();
            if text == if forward { open } else { close } {
                depth += 1;
            }
            if text == if forward { close } else { open } {
                depth -= 1;
                if depth == 0 {
                    return next;
                }
            }
            current = next;
        }
        pos
    }

    pub(super) fn viewport_position(
        &mut self,
        offset: ListOffset,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Option<Position> {
        let mut pos = self.boundary(offset.item_ix, true, count, load)?;
        if pos.row == offset.item_ix {
            let row = self.row(pos.row, load);
            pos.cell = row
                .cells
                .iter()
                .position(|cell| cell.bounds.bottom() > offset.offset_in_item)
                .unwrap_or(row.cells.len() - 1);
            pos.cell = row.line_cell(pos.cell, KeyboardCommand::ScreenFirst);
        }
        Some(pos)
    }

    pub(super) fn word_at(
        &mut self,
        pos: Position,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> String {
        let row = self.row(pos.row, load);
        let class = row.cells[pos.cell].class(false);
        if class == 0 {
            return String::new();
        }
        let mut start = pos.cell;
        let mut end = pos.cell + 1;
        while start > 0 && row.cells[start - 1].class(false) == class {
            start -= 1;
        }
        while end < row.cells.len() && row.cells[end].class(false) == class {
            end += 1;
        }
        row.cells[start..end]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect()
    }

    pub(super) fn search_motion(
        &mut self,
        pos: Position,
        forward: bool,
        count: usize,
        load: &mut impl FnMut(usize) -> TextRow,
    ) -> Position {
        if self.search.is_empty() {
            return pos;
        }
        let query = self.search.clone();
        let query_cells = query.graphemes(true).count();
        let whole_word = self.search_whole_word;
        let mut best = None;
        let mut wrap = None;
        for row_index in 0..count {
            let row = self.row(row_index, load);
            let text = row
                .cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>();
            let mut offset = 0;
            for (cell, value) in row.cells.iter().enumerate() {
                let ends = cell + query_cells;
                let whole_match = !whole_word
                    || ((cell == 0 || row.cells[cell - 1].class(false) != value.class(false))
                        && (ends >= row.cells.len()
                            || row.cells[ends].class(false) != row.cells[ends - 1].class(false)));
                if text[offset..].starts_with(&query) && whole_match {
                    let candidate = Position {
                        row: row_index,
                        cell,
                    };
                    if forward {
                        if wrap.is_none() {
                            wrap = Some(candidate);
                        }
                        if candidate > pos && best.is_none() {
                            best = Some(candidate);
                        }
                    } else {
                        wrap = Some(candidate);
                        if candidate < pos {
                            best = Some(candidate);
                        }
                    }
                }
                offset += value.text.len();
            }
            // A document search must not retain every virtualized row.
            if row_index != pos.row && !self.anchor.is_some_and(|p| p.row == row_index) {
                self.cache.remove(&row_index);
            }
        }
        let target = best.or(wrap).unwrap_or(pos);
        self.row(target.row, load);
        target
    }
}
