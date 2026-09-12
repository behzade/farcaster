use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SubmittedUser {
    pub(super) item: Arc<TranscriptItem>,
    pub(super) accepted: bool,
    pub(super) delivered: bool,
    pub(super) delivery_tracked: bool,
    pub(super) unknown: bool,
}

impl ConversationState {
    pub(crate) fn bind_submitted_prompt(&mut self, id: &str, item: &Arc<TranscriptItem>) {
        self.bind_submitted_prompt_with_evidence(id, item, false);
    }

    pub(crate) fn bind_submitted_prompt_with_evidence(
        &mut self,
        id: &str,
        item: &Arc<TranscriptItem>,
        delivery_tracked: bool,
    ) {
        self.submitted_users
            .entry(id.to_owned())
            .or_insert_with(|| SubmittedUser {
                item: item.clone(),
                accepted: false,
                delivered: false,
                delivery_tracked,
                unknown: false,
            });
    }

    /// Admission and delivery update one row. Neither is a new assistant stream.
    pub(crate) fn record_prompt_delivery(
        &mut self,
        id: &str,
        message: &Value,
        status: &str,
    ) -> Option<usize> {
        if id.is_empty() || !matches!(status, "accepted" | "delivered" | "unknown" | "rejected") {
            return None;
        }
        let previous = self.submitted_users.get(id).cloned();
        let index = previous
            .as_ref()
            .and_then(|entry| self.items.position(|item| Arc::ptr_eq(item, &entry.item)));
        if status == "rejected" {
            // A later error cannot undo proven receipt or a committed user item.
            if previous
                .as_ref()
                .is_none_or(|entry| entry.accepted || entry.delivered)
            {
                return None;
            }
            self.submitted_users.remove(id);
            if let Some(index) = index {
                let item = self.items[index].clone();
                self.rollback_local_user(&item);
            }
            return index;
        }
        let mut entry = if let Some(entry) = previous {
            entry
        } else {
            let optimistic = (message.get("queued").and_then(Value::as_bool) != Some(true))
                .then(|| self.optimistic_user.clone())
                .flatten();
            let item = optimistic.or_else(|| {
                project_message_items(message)
                    .into_iter()
                    .find(|item| item.kind == TranscriptKind::User)
                    .map(Arc::new)
            })?;
            SubmittedUser {
                item,
                accepted: false,
                delivered: false,
                delivery_tracked: message.get("deliveryTracked").and_then(Value::as_bool)
                    == Some(true),
                unknown: false,
            }
        };
        entry.delivery_tracked |=
            message.get("deliveryTracked").and_then(Value::as_bool) == Some(true);
        entry.accepted |= status == "accepted" || status == "delivered";
        entry.delivered |= status == "delivered";
        entry.unknown = !entry.accepted && status == "unknown";
        let index = index.or_else(|| self.items.position(|item| Arc::ptr_eq(item, &entry.item)));
        let was_optimistic = self
            .optimistic_user
            .as_ref()
            .is_some_and(|item| Arc::ptr_eq(item, &entry.item));
        let item = Arc::make_mut(&mut entry.item);
        item.label = if entry.accepted {
            String::new()
        } else {
            "Delivery unknown".into()
        };
        item.streaming = false;
        if was_optimistic {
            self.optimistic_user = Some(entry.item.clone());
        }
        let index = match index {
            Some(index) => {
                self.items.set(index, entry.item.clone());
                index
            }
            None => {
                let index = self.items.len();
                self.items.push(entry.item.clone());
                index
            }
        };
        self.submitted_users.insert(id.to_owned(), entry);
        Some(index)
    }
}
