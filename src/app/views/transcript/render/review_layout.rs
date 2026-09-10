//! Review handoff placement changes presentation, never conversation order.
use super::*;

pub(super) fn arrange(
    rows: PersistentVec<TranscriptRow>,
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    active_start: Option<usize>,
    completed_runs: &[std::ops::Range<usize>],
) -> PersistentVec<TranscriptRow> {
    if !rows
        .iter()
        .any(|row| matches!(row, TranscriptRow::Review { .. }))
    {
        return rows;
    }
    let mut output = PersistentVec::default();
    let mut span = Vec::new();
    for row in rows.iter().copied() {
        let index = row.item_start();
        // History has user-message boundaries. During a live run, steering
        // messages do not complete that run or relocate its existing reviews.
        let inside_completed = completed_runs
            .iter()
            .any(|run| run.start < index && index < run.end);
        let boundary = items
            .get(index)
            .is_some_and(|item| item.kind == TranscriptKind::User)
            && active_start.is_none_or(|start| index <= start)
            && !inside_completed;
        let run_boundary = active_start == Some(index)
            || completed_runs
                .iter()
                .any(|run| run.start == index || run.end == index);
        if (boundary || run_boundary)
            && span
                .last()
                .is_some_and(|last: &TranscriptRow| last.item_start() != index)
        {
            finish(&mut output, &span, items, active_start);
            span.clear();
        }
        span.push(row);
    }
    finish(&mut output, &span, items, active_start);
    output
}

fn finish(
    output: &mut PersistentVec<TranscriptRow>,
    span: &[TranscriptRow],
    items: &(impl Indexed<Arc<TranscriptItem>> + ?Sized),
    active_start: Option<usize>,
) {
    let working = active_start.is_some_and(|start| span.iter().any(|row| row.item_end() > start));
    let final_response = span.iter().rposition(|row| {
        items
            .get(row.item_start())
            .is_some_and(|item| item.kind == TranscriptKind::Assistant)
    });
    // A final response alone is the handoff, not evidence of further work.
    let last_work = span.iter().rposition(|row| {
        (row.item_start()..row.item_end()).any(|index| {
            items.get(index).is_some_and(|item| {
                matches!(item.kind, TranscriptKind::Tool | TranscriptKind::Thinking)
            })
        })
    });
    let mut handoffs = Vec::new();
    for (position, row) in span.iter().copied().enumerate() {
        if let TranscriptRow::Review {
            index, revision, ..
        } = row
        {
            let continued = last_work.is_some_and(|last| last > position);
            let review = TranscriptRow::Review {
                index,
                revision,
                working,
                continued,
            };
            if !working && final_response.is_some_and(|last| last > position) {
                handoffs.push(review);
            } else {
                output.push(review);
            }
        } else {
            output.push(row);
        }
        if Some(position) == final_response {
            output.extend(handoffs.drain(..));
        }
    }
}

#[cfg(test)]
#[path = "review_layout_tests.rs"]
mod tests;
