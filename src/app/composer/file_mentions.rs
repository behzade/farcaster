use std::{cell::RefCell, path::Path};

use nucleo_matcher::{
    Config, Matcher,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};

use crate::repository::{BackendPreference, RepositoryBackend};

const MAX_RESULTS: usize = 8;

thread_local! {
    static FILE_MATCHER: RefCell<Matcher> =
        RefCell::new(Matcher::new(Config::DEFAULT.match_paths()));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct MentionQuery {
    pub(in crate::app) range: std::ops::Range<usize>,
    pub(in crate::app) text: String,
}

pub(in crate::app) fn project_files(project: &Path, preference: BackendPreference) -> Vec<String> {
    RepositoryBackend::discover(project, preference)
        .ok()
        .flatten()
        .and_then(|backend| backend.list_project_files().ok())
        .unwrap_or_default()
}

pub(in crate::app) fn query_at_cursor(value: &str, cursor: usize) -> Option<MentionQuery> {
    if !value.is_char_boundary(cursor) {
        return None;
    }
    let prefix = &value[..cursor];
    let start = prefix
        .char_indices()
        .rev()
        .find_map(|(index, character)| (character == '@').then_some(index))?;
    if start > 0 && !value[..start].chars().next_back()?.is_whitespace() {
        return None;
    }
    let text = &value[start + 1..cursor];
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    Some(MentionQuery {
        range: start..cursor,
        text: text.to_owned(),
    })
}

pub(in crate::app) fn matches(files: &[String], query: &str) -> Vec<String> {
    let _timing = crate::app::infrastructure::performance::OperationTiming::new(
        crate::app::infrastructure::performance::OperationKind::FileMentionMatch,
        files.len(),
    );
    let pattern = Atom::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );
    let mut matches =
        FILE_MATCHER.with(|matcher| pattern.match_list(files, &mut matcher.borrow_mut()));
    matches.sort_by(|(left, left_score), (right, right_score)| {
        right_score.cmp(left_score).then_with(|| left.cmp(right))
    });
    matches
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(path, _)| path.clone())
        .collect()
}

pub(in crate::app) fn insert(value: &str, query: &MentionQuery, path: &str) -> (String, usize) {
    let replacement = format!("@{path} ");
    let mut text = value.to_owned();
    text.replace_range(query.range.clone(), &replacement);
    let cursor = query.range.start + replacement.len();
    (text, cursor)
}

#[cfg(test)]
#[path = "file_mentions_tests.rs"]
mod tests;
