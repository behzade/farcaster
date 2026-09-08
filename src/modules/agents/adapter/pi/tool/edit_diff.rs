//! Convert Pi's numbered display diff to a positioned, backend-neutral patch.
pub(super) fn unified_diff(diff: &str) -> Option<String> {
    let mut output = String::new();
    let mut block = Block::default();
    let mut delta = 0isize;
    for line in diff.lines() {
        let (sign, rest) = line.split_at_checked(1)?;
        if sign == " " {
            block.flush(&mut output, &mut delta)?;
            continue;
        }
        if sign != "+" && sign != "-" {
            return None;
        }
        let (number, text) = rest.trim_start().split_once(' ')?;
        let number: usize = number.parse().ok()?;
        if number == 0 {
            return None;
        }
        let (start, lines) = if sign == "-" {
            (&mut block.old_start, &mut block.old)
        } else {
            (&mut block.new_start, &mut block.new)
        };
        let first = *start.get_or_insert(number);
        if first.checked_add(lines.len())? != number {
            return None;
        }
        lines.push(text.to_owned());
    }
    block.flush(&mut output, &mut delta)?;
    (!output.is_empty()).then_some(output)
}

#[derive(Default)]
struct Block {
    old_start: Option<usize>,
    new_start: Option<usize>,
    old: Vec<String>,
    new: Vec<String>,
}

impl Block {
    fn flush(&mut self, output: &mut String, delta: &mut isize) -> Option<()> {
        if self.old.is_empty() && self.new.is_empty() {
            return Some(());
        }
        let old_start = match self.old_start {
            Some(start) => start,
            None => self.new_start?.checked_add_signed(-*delta)?,
        };
        let new_start = old_start.checked_add_signed(*delta)?;
        if self.new_start.is_some_and(|start| start != new_start) {
            return None;
        }
        output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start.checked_sub(usize::from(self.old.is_empty()))?,
            self.old.len(),
            new_start.checked_sub(usize::from(self.new.is_empty()))?,
            self.new.len()
        ));
        for (sign, lines) in [('-', &self.old), ('+', &self.new)] {
            for line in lines {
                output.push(sign);
                output.push_str(line);
                output.push('\n');
            }
        }
        *delta += self.new.len() as isize - self.old.len() as isize;
        *self = Self::default();
        Some(())
    }
}

#[cfg(test)]
#[path = "edit_diff_tests.rs"]
mod tests;
