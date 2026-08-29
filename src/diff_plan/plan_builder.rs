// Row projection and similarity alignment adapted from Pierre's
// iterateOverDiff.ts and realignChangeContent.ts.

use super::{
    DiffLayout, DiffLineKind, DiffPlanCell, DiffPlanLine, DiffPlanOptions, DiffPlanRow,
    FileChangeKind, FileDiffPlan, word_diff::changed_ranges,
};

#[derive(Clone, Debug)]
pub(super) struct RawCell {
    pub line_number: u64,
    pub text: String,
    pub no_newline: bool,
}

impl RawCell {
    pub(super) fn new(line_number: u64, text: String) -> Self {
        Self {
            line_number,
            text,
            no_newline: false,
        }
    }
}

pub(super) struct FilePlanner {
    options: DiffPlanOptions,
    pub plan: FileDiffPlan,
}

impl FilePlanner {
    pub fn new(options: DiffPlanOptions) -> Self {
        Self {
            options,
            plan: FileDiffPlan {
                old_path: None,
                path: None,
                kind: FileChangeKind::Changed,
                additions: 0,
                deletions: 0,
                rows: Vec::new(),
                omitted_rows: 0,
            },
        }
    }

    pub fn push(&mut self, row: DiffPlanRow) -> bool {
        let retained = self.retains_next_row();
        if retained {
            self.plan.rows.push(row);
        } else {
            self.plan.omitted_rows = self.plan.omitted_rows.saturating_add(1);
        }
        retained
    }

    pub fn push_context(&mut self, old_number: u64, new_number: u64, text: String) -> bool {
        let new = RawCell::new(new_number, text);
        let old = (self.options.layout == DiffLayout::Split)
            .then(|| RawCell::new(old_number, new.text.clone()));
        self.push(DiffPlanRow::Line(DiffPlanLine {
            kind: DiffLineKind::Context,
            old: old.map(|old| cell(old, Vec::new())),
            new: Some(cell(new, Vec::new())),
        }))
    }

    pub fn push_changes(&mut self, old: Vec<RawCell>, new: Vec<RawCell>) {
        if old.is_empty() && new.is_empty() {
            return;
        }
        let total_rows = match self.options.layout {
            DiffLayout::Unified => old.len().saturating_add(new.len()),
            DiffLayout::Split => old.len().max(new.len()),
        };
        let retained_rows = self.remaining_rows().min(total_rows);
        if retained_rows > 0 {
            let alignment = Alignment::new(&old, &new);
            match self.options.layout {
                DiffLayout::Unified => {
                    self.push_unified_changes(&old, &new, alignment, retained_rows);
                }
                DiffLayout::Split => {
                    self.push_split_changes(&old, &new, alignment, retained_rows);
                }
            }
        }
        self.plan.omitted_rows = self
            .plan
            .omitted_rows
            .saturating_add(total_rows.saturating_sub(retained_rows));
    }

    pub fn mark_retained_context_no_newline(&mut self) {
        let Some(DiffPlanRow::Line(line)) = self.plan.rows.last_mut() else {
            return;
        };
        if let Some(old) = line.old.as_mut() {
            old.no_newline = true;
        }
        if let Some(new) = line.new.as_mut() {
            new.no_newline = true;
        }
    }

    fn push_unified_changes(
        &mut self,
        old: &[RawCell],
        new: &[RawCell],
        alignment: Alignment,
        retained_rows: usize,
    ) {
        let retained_old = old.len().min(retained_rows);
        let retained_new = retained_rows.saturating_sub(retained_old).min(new.len());
        let mut old_ranges = vec![Vec::new(); retained_old];
        let mut new_ranges = vec![Vec::new(); retained_new];
        if self.options.intraline_changes {
            for (old_index, new_index) in alignment.pairs() {
                let (Some(old_index), Some(new_index)) = (old_index, new_index) else {
                    continue;
                };
                if old_index >= retained_old && new_index >= retained_new {
                    continue;
                }
                let (old_changed, new_changed) = changed_ranges(
                    &old[old_index].text,
                    &new[new_index].text,
                    self.options.max_intraline_bytes,
                );
                if let Some(ranges) = old_ranges.get_mut(old_index) {
                    *ranges = old_changed;
                }
                if let Some(ranges) = new_ranges.get_mut(new_index) {
                    *ranges = new_changed;
                }
            }
        }
        for (old, changed) in old.iter().take(retained_old).cloned().zip(old_ranges) {
            self.plan.rows.push(DiffPlanRow::Line(DiffPlanLine {
                kind: DiffLineKind::Change,
                old: Some(cell(old, changed)),
                new: None,
            }));
        }
        for (new, changed) in new.iter().take(retained_new).cloned().zip(new_ranges) {
            self.plan.rows.push(DiffPlanRow::Line(DiffPlanLine {
                kind: DiffLineKind::Change,
                old: None,
                new: Some(cell(new, changed)),
            }));
        }
    }

    fn push_split_changes(
        &mut self,
        old: &[RawCell],
        new: &[RawCell],
        alignment: Alignment,
        retained_rows: usize,
    ) {
        for (old_index, new_index) in alignment.pairs().take(retained_rows) {
            let old = old_index.and_then(|index| old.get(index));
            let new = new_index.and_then(|index| new.get(index));
            let (old_changed, new_changed) = self.changed_for_pair(old, new);
            self.plan.rows.push(DiffPlanRow::Line(DiffPlanLine {
                kind: DiffLineKind::Change,
                old: old.cloned().map(|old| cell(old, old_changed)),
                new: new.cloned().map(|new| cell(new, new_changed)),
            }));
        }
    }

    fn changed_for_pair(
        &self,
        old: Option<&RawCell>,
        new: Option<&RawCell>,
    ) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
        if self.options.intraline_changes
            && let (Some(old), Some(new)) = (old, new)
        {
            changed_ranges(&old.text, &new.text, self.options.max_intraline_bytes)
        } else {
            (Vec::new(), Vec::new())
        }
    }

    fn retains_next_row(&self) -> bool {
        self.remaining_rows() > 0
    }

    fn remaining_rows(&self) -> usize {
        self.options
            .max_rows_per_file
            .map_or(usize::MAX, |maximum| {
                maximum.saturating_sub(self.plan.rows.len())
            })
    }
}

#[derive(Clone, Copy)]
struct Alignment {
    old_len: usize,
    new_len: usize,
    offset: usize,
}

impl Alignment {
    fn new(old: &[RawCell], new: &[RawCell]) -> Self {
        Self {
            old_len: old.len(),
            new_len: new.len(),
            offset: best_alignment_offset(old, new),
        }
    }

    fn pairs(self) -> impl Iterator<Item = (Option<usize>, Option<usize>)> {
        let pair_count = self.old_len.min(self.new_len);
        (0..self.old_len.max(self.new_len)).map(move |row| {
            if self.new_len > self.old_len {
                if row < self.offset {
                    (None, Some(row))
                } else if row < self.offset + pair_count {
                    (Some(row - self.offset), Some(row))
                } else {
                    (None, Some(row))
                }
            } else if row < self.offset {
                (Some(row), None)
            } else if row < self.offset + pair_count {
                (Some(row), Some(row - self.offset))
            } else {
                (Some(row), None)
            }
        })
    }
}

// Pierre realigns only decisive improvements and bounds pathological scans.
fn best_alignment_offset(old: &[RawCell], new: &[RawCell]) -> usize {
    const MAX_COMPARISONS: usize = 4_096;
    const MIN_IMPROVEMENT_PER_PAIR: f64 = 0.5;

    let pair_count = old.len().min(new.len());
    let surplus = old.len().abs_diff(new.len());
    if pair_count == 0
        || surplus == 0
        || pair_count.saturating_mul(surplus.saturating_add(1)) > MAX_COMPARISONS
    {
        return 0;
    }
    let old = old
        .iter()
        .map(|line| strip_whitespace(&line.text))
        .collect::<Vec<_>>();
    let new = new
        .iter()
        .map(|line| strip_whitespace(&line.text))
        .collect::<Vec<_>>();
    let new_is_longer = new.len() > old.len();
    let score_at = |offset| {
        let (old_start, new_start) = if new_is_longer {
            (0, offset)
        } else {
            (offset, 0)
        };
        old[old_start..]
            .iter()
            .zip(&new[new_start..])
            .take(pair_count)
            .map(|(old, new)| line_similarity(old, new))
            .sum::<f64>()
    };
    let mut best_offset = 0;
    let mut best_score = score_at(0) + pair_count as f64 * MIN_IMPROVEMENT_PER_PAIR;
    for offset in 1..=surplus {
        let score = score_at(offset);
        if score > best_score {
            best_score = score;
            best_offset = offset;
        }
    }
    best_offset
}

fn line_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    let left = left.as_bytes();
    let right = right.as_bytes();
    let minimum = left.len().min(right.len());
    let maximum = left.len().max(right.len());
    if minimum == 0 {
        return 0.0;
    }
    let prefix = left
        .iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    (prefix.saturating_add(suffix)) as f64 / maximum as f64
}

fn strip_whitespace(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn cell(raw: RawCell, changed: Vec<std::ops::Range<usize>>) -> DiffPlanCell {
    DiffPlanCell {
        line_number: raw.line_number,
        text: raw.text,
        changed,
        no_newline: raw.no_newline,
    }
}
