use super::*;
use farcaster_agent_protocol::{DeliveredMessage, DeliveryStatus, extensions::PromptMode};

#[derive(Clone, Debug, PartialEq)]
pub struct PendingReceipt {
    pub id: String,
    pub mode: Option<PromptMode>,
    pub text: String,
    pub images: Arc<Vec<Arc<EncodedImage>>>,
    pub unknown: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SubmittedUser {
    pub(super) item: Arc<TranscriptItem>,
    pub(super) accepted: bool,
    pub(super) delivered: bool,
    pub(super) delivery_tracked: bool,
    pub(super) unknown: bool,
    pub(super) queued: bool,
    mode: Option<PromptMode>,
    order: usize,
}

impl SubmittedUser {
    pub(super) fn is_visible(&self) -> bool {
        !self.queued || self.delivered
    }
}

impl ConversationState {
    pub fn dismiss_pending_receipt(&mut self, id: &str) {
        if self
            .submitted_users
            .get(id)
            .is_some_and(|entry| !entry.delivered && entry.queued)
        {
            self.submitted_users.remove(id);
        }
    }

    pub fn pending_receipts(&self) -> Vec<PendingReceipt> {
        let mut pending = self
            .submitted_users
            .iter()
            .filter(|(_, entry)| !entry.is_visible())
            .collect::<Vec<_>>();
        pending.sort_by_key(|(_, entry)| entry.order);
        pending
            .into_iter()
            .map(|(id, entry)| PendingReceipt {
                id: id.clone(),
                mode: entry.mode,
                text: entry.item.text.clone(),
                images: entry.item.images.clone(),
                unknown: entry.unknown,
            })
            .collect()
    }

    pub fn bind_submitted_prompt(&mut self, id: &str, item: &Arc<TranscriptItem>) {
        self.bind_submitted_prompt_with_evidence(id, item, false);
    }

    pub fn bind_submitted_prompt_with_evidence(
        &mut self,
        id: &str,
        item: &Arc<TranscriptItem>,
        delivery_tracked: bool,
    ) {
        let order = self.next_submission_order;
        self.next_submission_order = order.saturating_add(1);
        self.submitted_users
            .entry(id.to_owned())
            .or_insert_with(|| SubmittedUser {
                item: item.clone(),
                accepted: false,
                delivered: false,
                delivery_tracked,
                unknown: false,
                queued: false,
                mode: Some(PromptMode::Normal),
                order,
            });
    }

    pub fn record_prompt_delivery(
        &mut self,
        id: &str,
        message: Option<&DeliveredMessage>,
        status: DeliveryStatus,
    ) -> Option<usize> {
        if status == DeliveryStatus::Cancelled {
            self.dismiss_pending_receipt(id);
            return None;
        }
        if id.is_empty() {
            return None;
        }
        let previous = self.submitted_users.get(id).cloned();
        let index = previous
            .as_ref()
            .and_then(|entry| self.items.position(|item| Arc::ptr_eq(item, &entry.item)));
        if status == DeliveryStatus::Rejected {
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
            let order = self.next_submission_order;
            self.next_submission_order = order.saturating_add(1);
            let optimistic = (!message.is_some_and(|message| message.queued))
                .then(|| self.optimistic_user.clone())
                .flatten();
            let item = optimistic.or_else(|| {
                project_message_items(&message?.value())
                    .into_iter()
                    .find(|item| item.kind == TranscriptKind::User)
                    .map(Arc::new)
            })?;
            SubmittedUser {
                item,
                accepted: false,
                delivered: false,
                delivery_tracked: message.is_some_and(|message| message.delivery_tracked),
                unknown: false,
                queued: message.is_some_and(|message| message.queued),
                mode: message.and_then(|message| message.prompt_mode),
                order,
            }
        };
        entry.delivery_tracked |= message.is_some_and(|message| message.delivery_tracked);
        entry.accepted |= status == DeliveryStatus::Accepted || status == DeliveryStatus::Delivered;
        entry.delivered |= status == DeliveryStatus::Delivered;
        entry.unknown = !entry.accepted && status == DeliveryStatus::Unknown;
        if !entry.is_visible() {
            self.submitted_users.insert(id.to_owned(), entry);
            return None;
        }
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
