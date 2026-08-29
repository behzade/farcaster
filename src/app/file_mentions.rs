//! Repository file discovery and matching for composer `@` mentions.

use std::path::Path;

use crate::repository::{BackendPreference, RepositoryBackend};

const MAX_RESULTS: usize = 8;

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
    let _timing = crate::performance::OperationTiming::new(
        crate::performance::OperationKind::FileMentionMatch,
        files.len(),
    );
    let query = query.to_lowercase();
    let mut matches = files
        .iter()
        .filter_map(|path| fuzzy_score(&path.to_lowercase(), &query).map(|score| (score, path)))
        .collect::<Vec<_>>();
    matches.sort_by(|(left_score, left), (right_score, right)| {
        right_score.cmp(left_score).then_with(|| left.cmp(right))
    });
    matches
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, path)| path.clone())
        .collect()
}

pub(super) fn insert(value: &str, query: &MentionQuery, path: &str) -> (String, usize) {
    let replacement = format!("@{path} ");
    let mut text = value.to_owned();
    text.replace_range(query.range.clone(), &replacement);
    let cursor = query.range.start + replacement.len();
    (text, cursor)
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score: usize = 0;
    let mut previous = None;
    let mut candidate_chars = candidate.char_indices();
    for needle in query.chars() {
        let (index, _) = candidate_chars.find(|(_, character)| *character == needle)?;
        score += 10;
        if previous.is_some_and(|previous| index == previous + 1) {
            score += 8;
        }
        if index == 0 || candidate.as_bytes().get(index.wrapping_sub(1)) == Some(&b'/') {
            score += 5;
        }
        previous = Some(index);
    }
    if candidate
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains(query))
    {
        score += 20;
    }
    Some(score.saturating_sub(candidate.len() / 20))
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
    fn insertion_replaces_only_the_active_token_and_tracks_byte_cursor() {
        let query = query_at_cursor("🙂 see @ma now", 12).expect("query");
        let (text, cursor) = insert("🙂 see @ma now", &query, "src/main.rs");
        assert_eq!(text, "🙂 see @src/main.rs  now");
        assert_eq!(cursor, 22);
    }
}
