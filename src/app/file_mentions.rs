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
pub(super) struct MentionQuery {
    pub(super) range: std::ops::Range<usize>,
    pub(super) text: String,
}

pub(super) fn project_files(project: &Path, preference: BackendPreference) -> Vec<String> {
    RepositoryBackend::discover(project, preference)
        .ok()
        .flatten()
        .and_then(|backend| backend.list_project_files().ok())
        .unwrap_or_default()
}

pub(super) fn query_at_cursor(value: &str, cursor: usize) -> Option<MentionQuery> {
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

pub(super) fn matches(files: &[String], query: &str) -> Vec<String> {
    let _timing = crate::app::performance::OperationTiming::new(
        crate::app::performance::OperationKind::FileMentionMatch,
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

pub(super) fn insert(value: &str, query: &MentionQuery, path: &str) -> (String, usize) {
    let replacement = format!("@{path} ");
    let mut text = value.to_owned();
    text.replace_range(query.range.clone(), &replacement);
    let cursor = query.range.start + replacement.len();
    (text, cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_query_uses_the_token_at_the_cursor() {
        assert_eq!(
            query_at_cursor("read @src/ma please", 12),
            Some(MentionQuery {
                range: 5..12,
                text: "src/ma".into(),
            })
        );
        assert!(query_at_cursor("email@example", 13).is_none());
        assert!(query_at_cursor("@two words", 10).is_none());
    }

    #[test]
    fn matching_is_fuzzy_and_prefers_file_names() {
        let files = vec![
            "src/main.rs".into(),
            "docs/runtime.md".into(),
            "main.txt".into(),
        ];
        assert_eq!(matches(&files, "main"), ["main.txt", "src/main.rs"]);
        assert_eq!(matches(&files, "srm"), ["src/main.rs", "docs/runtime.md"]);
    }

    #[test]
    fn empty_matching_keeps_every_file_in_stable_order() {
        let files = vec!["z.rs".into(), "a.rs".into()];
        assert_eq!(matches(&files, ""), ["a.rs", "z.rs"]);
    }

    #[test]
    fn unicode_matching_uses_character_boundaries() {
        let files = vec!["src/café.rs".into(), "src/cafeteria.rs".into()];
        assert_eq!(matches(&files, "café"), ["src/café.rs"]);
    }

    #[test]
    fn insertion_replaces_only_the_active_token_and_tracks_byte_cursor() {
        let query = query_at_cursor("🙂 see @ma now", 12).expect("query");
        let (text, cursor) = insert("🙂 see @ma now", &query, "src/main.rs");
        assert_eq!(text, "🙂 see @src/main.rs  now");
        assert_eq!(cursor, 22);
    }
}
