/// Reconcile a completed snapshot with text already emitted at the same scope.
/// Callers retain native block identity and decide whether that scope can be replaced.
pub(super) enum TextUpdate<'a> {
    Unchanged,
    Append(&'a str),
    Replace(&'a str),
}

impl<'a> TextUpdate<'a> {
    pub(super) fn between(streamed: &str, completed: &'a str) -> Self {
        if completed.is_empty() || completed == streamed {
            Self::Unchanged
        } else if let Some(suffix) = completed.strip_prefix(streamed) {
            Self::Append(suffix)
        } else {
            Self::Replace(completed)
        }
    }
}
