// Pierre uses jsdiff's word diff and then joins one-space gaps between adjacent
// changed spans. This renderer-neutral rewrite uses a bounded token LCS and the
// same visual joining rule, avoiding an additional runtime dependency.

use std::ops::Range;

const MAX_TOKEN_PRODUCT: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind {
    Word,
    Whitespace,
    Punctuation,
}

#[derive(Clone, Debug)]
struct Token<'a> {
    range: Range<usize>,
    value: &'a str,
}

pub(super) fn changed_ranges(
    old: &str,
    new: &str,
    max_bytes: usize,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old == new {
        return (Vec::new(), Vec::new());
    }
    if old.len() > max_bytes || new.len() > max_bytes {
        return (Vec::new(), Vec::new());
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    if old_tokens.len().saturating_mul(new_tokens.len()) > MAX_TOKEN_PRODUCT {
        return (Vec::new(), Vec::new());
    }

    let matches = lcs_matches(&old_tokens, &new_tokens);
    let mut old_matched = vec![false; old_tokens.len()];
    let mut new_matched = vec![false; new_tokens.len()];
    for (old_index, new_index) in matches {
        old_matched[old_index] = true;
        new_matched[new_index] = true;
    }
    (
        ranges_for_unmatched(old, &old_tokens, &old_matched),
        ranges_for_unmatched(new, &new_tokens, &new_matched),
    )
}

fn tokenize(value: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut previous = None;
    for (index, character) in value.char_indices() {
        let kind = token_kind(character);
        if previous.is_some_and(|previous| previous != kind)
            || kind == TokenKind::Punctuation && index > start
        {
            tokens.push(Token {
                range: start..index,
                value: &value[start..index],
            });
            start = index;
        }
        if kind == TokenKind::Punctuation {
            let end = index + character.len_utf8();
            tokens.push(Token {
                range: index..end,
                value: &value[index..end],
            });
            start = end;
            previous = None;
        } else {
            previous = Some(kind);
        }
    }
    if start < value.len() {
        tokens.push(Token {
            range: start..value.len(),
            value: &value[start..],
        });
    }
    tokens
}

fn token_kind(character: char) -> TokenKind {
    if character.is_whitespace() {
        TokenKind::Whitespace
    } else if character.is_alphanumeric() || character == '_' {
        TokenKind::Word
    } else {
        TokenKind::Punctuation
    }
}

fn lcs_matches(old: &[Token<'_>], new: &[Token<'_>]) -> Vec<(usize, usize)> {
    let width = new.len().saturating_add(1);
    let mut lengths = vec![0_u16; old.len().saturating_add(1).saturating_mul(width)];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            let index = old_index * width + new_index;
            lengths[index] = if old[old_index].value == new[new_index].value {
                lengths[(old_index + 1) * width + new_index + 1].saturating_add(1)
            } else {
                lengths[(old_index + 1) * width + new_index]
                    .max(lengths[old_index * width + new_index + 1])
            };
        }
    }

    let mut matches = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < old.len() && new_index < new.len() {
        if old[old_index].value == new[new_index].value {
            matches.push((old_index, new_index));
            old_index += 1;
            new_index += 1;
        } else if lengths[(old_index + 1) * width + new_index]
            >= lengths[old_index * width + new_index + 1]
        {
            old_index += 1;
        } else {
            new_index += 1;
        }
    }
    matches
}

fn ranges_for_unmatched(source: &str, tokens: &[Token<'_>], matched: &[bool]) -> Vec<Range<usize>> {
    let mut ranges = Vec::<Range<usize>>::new();
    for (token, matched) in tokens.iter().zip(matched) {
        if *matched {
            continue;
        }
        if let Some(previous) = ranges.last_mut() {
            let gap = &source[previous.end..token.range.start];
            if previous.end == token.range.start
                || gap.chars().count() == 1 && gap.chars().all(char::is_whitespace)
            {
                previous.end = token.range.end;
                continue;
            }
        }
        ranges.push(token.range.clone());
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(range: Range<usize>) -> Vec<Range<usize>> {
        vec![range]
    }

    #[test]
    fn changed_word_is_precise_on_both_sides() {
        assert_eq!(
            changed_ranges("let answer = 41;", "let answer = 42;", 2_000),
            (one(13..15), one(13..15))
        );
    }

    #[test]
    fn oversized_lines_skip_intraline_ranges() {
        assert_eq!(
            changed_ranges("old value", "new value", 3),
            (vec![], vec![])
        );
    }

    #[test]
    fn inserted_text_only_marks_the_new_side() {
        assert_eq!(changed_ranges("", "new", 2_000), (vec![], one(0..3)));
    }

    #[test]
    fn one_space_between_changes_uses_one_visual_span() {
        let (old, new) = changed_ranges("let a old", "let b new", 2_000);

        assert_eq!(old, one(4..9));
        assert_eq!(new, one(4..9));
    }

    #[test]
    fn unicode_changes_preserve_the_unchanged_prefix() {
        for (prefix, old, new) in [
            ("let value = ", "let value = café", "let value = 茶"),
            ("سلام ", "سلام دنیا", "سلام جهان"),
            ("status ", "status 😀", "status ✅"),
        ] {
            assert_eq!(
                changed_ranges(old, new, 2_000),
                (one(prefix.len()..old.len()), one(prefix.len()..new.len()))
            );
        }
    }
}
