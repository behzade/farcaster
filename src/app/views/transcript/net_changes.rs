//! Compose recorded line edits without reading the current working copy.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    /// Zero-based position in the file after preceding edits have been applied.
    pub start: usize,
    pub old: Vec<String>,
    pub new: Vec<String>,
}

enum Line {
    Original(usize),
    Added(String),
}

#[derive(Default)]
pub(super) struct NetChanges {
    original: Vec<Option<String>>,
    current: Vec<Line>,
}

impl NetChanges {
    pub fn apply(&mut self, edits: &[Edit]) -> Option<()> {
        for edit in edits {
            let end = edit.start.checked_add(edit.old.len())?;
            // Bound work for malformed or unusually large transcript patches.
            if end.max(self.current.len()).checked_add(edit.new.len())? > 200_000 {
                return None;
            }
            while self.current.len() < end {
                self.current.push(Line::Original(self.original.len()));
                self.original.push(None);
            }
            for (line, expected) in self.current[edit.start..end].iter().zip(&edit.old) {
                match line {
                    Line::Original(index) => {
                        let text = &mut self.original[*index];
                        if text.as_ref().is_some_and(|text| text != expected) {
                            return None;
                        }
                        *text = Some(expected.clone());
                    }
                    Line::Added(text) if text != expected => return None,
                    Line::Added(_) => {}
                }
            }
            self.current
                .splice(edit.start..end, edit.new.iter().cloned().map(Line::Added));
        }
        Some(())
    }

    pub fn counts(&self) -> Option<(usize, usize)> {
        let mut counts = (0, 0);
        let mut cursor = 0;
        let mut added = Vec::new();
        let mut budget = 4_000_000usize;
        for line in self
            .current
            .iter()
            .chain(std::iter::once(&Line::Original(self.original.len())))
        {
            match line {
                Line::Added(text) => added.push(text.as_str()),
                Line::Original(index) => {
                    let old = self.original[cursor..*index]
                        .iter()
                        .map(|text| text.as_deref())
                        .collect::<Option<Vec<_>>>()?;
                    let common = common_lines(&old, &added, &mut budget)?;
                    counts.0 += added.len() - common;
                    counts.1 += old.len() - common;
                    added.clear();
                    cursor = index + 1;
                }
            }
        }
        Some(counts)
    }
}

fn common_lines(old: &[&str], new: &[&str], budget: &mut usize) -> Option<usize> {
    *budget = budget.checked_sub(old.len().checked_mul(new.len())?)?;
    let mut lengths = vec![0; new.len() + 1];
    for before in old {
        let mut diagonal = 0;
        for (index, after) in new.iter().enumerate() {
            let previous = lengths[index + 1];
            lengths[index + 1] = if before == after {
                diagonal + 1
            } else {
                lengths[index + 1].max(lengths[index])
            };
            diagonal = previous;
        }
    }
    lengths.last().copied()
}

pub(super) fn unified_edits(diff: &str) -> Option<Vec<Edit>> {
    let mut lines = diff.lines().peekable();
    let mut edits = Vec::new();
    let mut delta = 0isize;
    let mut previous_end = 0;
    while let Some(line) = lines.next() {
        if !line.starts_with("@@ ") {
            if edits.is_empty()
                && (line.starts_with("diff ")
                    || line.starts_with("index ")
                    || line.starts_with("--- ")
                    || line.starts_with("+++ "))
            {
                continue;
            }
            return None;
        }
        let mut header = line.split_whitespace();
        header.next()?;
        let (old_start, old_count) = range(header.next()?.strip_prefix('-')?)?;
        let (new_start, new_count) = range(header.next()?.strip_prefix('+')?)?;
        if header.next()? != "@@" {
            return None;
        }
        let mut edit = Edit {
            start: new_start.checked_sub(usize::from(new_count > 0))?,
            old: Vec::new(),
            new: Vec::new(),
        };
        let old_index = old_start.checked_sub(usize::from(old_count > 0))?;
        if old_index < previous_end || old_index.checked_add_signed(delta)? != edit.start {
            return None;
        }
        previous_end = old_index.checked_add(old_count)?;
        delta = delta
            .checked_add(isize::try_from(new_count).ok()?)?
            .checked_sub(isize::try_from(old_count).ok()?)?;
        while edit.old.len() < old_count || edit.new.len() < new_count {
            let line = lines.next()?;
            let (sign, text) = line.split_at_checked(1)?;
            match sign {
                " " => {
                    edit.old.push(text.into());
                    edit.new.push(text.into());
                }
                "-" => edit.old.push(text.into()),
                "+" => edit.new.push(text.into()),
                "\\" if line == "\\ No newline at end of file" => continue,
                _ => return None,
            }
            if edit.old.len() > old_count || edit.new.len() > new_count {
                return None;
            }
        }
        if lines.peek() == Some(&"\\ No newline at end of file") {
            lines.next();
        }
        edits.push(edit);
    }
    (!edits.is_empty()).then_some(edits)
}

fn range(value: &str) -> Option<(usize, usize)> {
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

#[cfg(test)]
#[path = "net_changes_tests.rs"]
mod tests;
