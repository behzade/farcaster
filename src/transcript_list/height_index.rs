use std::ops::Range;

use gpui::{Pixels, px};

#[derive(Clone, Copy)]
struct RowHeight {
    value: Pixels,
    measured: bool,
}

impl RowHeight {
    fn estimated(value: Pixels) -> Self {
        Self {
            value: value.max(px(1.0)),
            measured: false,
        }
    }
}

#[derive(Default)]
pub(super) struct HeightIndex {
    rows: Vec<RowHeight>,
    fenwick: Vec<f32>,
}

impl HeightIndex {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn splice(
        &mut self,
        old_range: Range<usize>,
        size_hints: impl IntoIterator<Item = Pixels>,
    ) -> usize {
        let replacement = size_hints
            .into_iter()
            .map(RowHeight::estimated)
            .collect::<Vec<_>>();
        let replacement_len = replacement.len();
        let appending =
            !self.is_empty() && old_range.start == self.len() && old_range.end == self.len();
        if appending {
            self.extend(replacement);
        } else {
            self.rows.splice(old_range, replacement);
            self.rebuild();
        }
        replacement_len
    }

    fn rebuild(&mut self) {
        self.fenwick = vec![0.0; self.len() + 1];
        for (row, height) in self.rows.iter().enumerate() {
            let slot = row + 1;
            self.fenwick[slot] += f32::from(height.value);
            let parent = slot + (slot & slot.wrapping_neg());
            if parent < self.fenwick.len() {
                self.fenwick[parent] += self.fenwick[slot];
            }
        }
    }

    fn extend(&mut self, rows: impl IntoIterator<Item = RowHeight>) {
        for row in rows {
            let slot = self.len() + 1;
            let block_start = slot - (slot & slot.wrapping_neg());
            let preceding = self.prefix(slot - 1) - self.prefix(block_start);
            self.rows.push(row);
            self.fenwick.push(f32::from(preceding + row.value));
        }
    }

    pub(super) fn set_height(&mut self, row: usize, height: Pixels) {
        let height = height.max(px(1.0));
        let previous = self.rows[row].value;
        self.rows[row] = RowHeight {
            value: height,
            measured: true,
        };
        let mut slot = row + 1;
        while slot < self.fenwick.len() {
            self.fenwick[slot] += f32::from(height - previous);
            slot += slot & slot.wrapping_neg();
        }
    }

    pub(super) fn invalidate(&mut self, range: Range<usize>) {
        if let Some(rows) = self.rows.get_mut(range) {
            rows.iter_mut().for_each(|row| row.measured = false);
        }
    }

    pub(super) fn invalidate_all(&mut self) {
        self.rows.iter_mut().for_each(|row| row.measured = false);
    }

    pub(super) fn is_measured(&self, row: usize) -> bool {
        self.rows[row].measured
    }

    pub(super) fn height(&self, row: usize) -> Option<Pixels> {
        self.rows.get(row).map(|row| row.value)
    }

    pub(super) fn prefix(&self, end: usize) -> Pixels {
        let mut slot = end.min(self.len());
        let mut height = 0.0;
        while slot > 0 {
            height += self.fenwick[slot];
            slot &= slot - 1;
        }
        px(height)
    }

    pub(super) fn total(&self) -> Pixels {
        self.prefix(self.len())
    }

    pub(super) fn row_at(&self, offset: Pixels) -> usize {
        let target = f32::from(offset.max(px(0.0)));
        let mut index = 0;
        let mut prefix = 0.0;
        let mut step = self.len().next_power_of_two();
        while step > 0 {
            let next = index + step;
            if next < self.fenwick.len() && prefix + self.fenwick[next] <= target {
                index = next;
                prefix += self.fenwick[next];
            }
            step >>= 1;
        }
        index.min(self.len())
    }
}
