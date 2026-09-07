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
mod tests {
    use super::*;
    #[test]
    fn keeps_indentation_and_positions_after_insertions() {
        assert_eq!(
            unified_diff(
                " 1 context\n-2 old\n+2   new\n+3 extra\n 3 tail\n   ...\n-9 later\n+10 last"
            ),
            Some("@@ -2,1 +2,2 @@\n-old\n+  new\n+extra\n@@ -9,1 +10,1 @@\n-later\n+last\n".into())
        );
    }
    #[test]
    fn insertions_and_deletions_use_empty_range_positions() {
        assert_eq!(
            unified_diff("+1 new"),
            Some("@@ -0,0 +1,1 @@\n+new\n".into())
        );
        assert_eq!(
            unified_diff("-1 old"),
            Some("@@ -1,1 +0,0 @@\n-old\n".into())
        );
        assert_eq!(unified_diff("-1 old\n+3 wrong"), None);
    }
}
