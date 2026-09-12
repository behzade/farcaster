use super::*;

#[derive(Clone)]
pub(super) struct RetiredPrompt {
    pub(super) outbox_id: Option<i64>,
    target: String,
    pub(super) session: Option<PathBuf>,
    delivery_tracked: bool,
    pub(super) delivered: bool,
}

impl RuntimeOwner {
    pub(super) fn retire_unknown_prompt(&mut self, id: &str) {
        self.retired_prompts
            .entry(id.to_owned())
            .or_insert(RetiredPrompt {
                outbox_id: self.pending_outbox_id.take(),
                target: self.pending_prompt_target.clone().unwrap_or_default(),
                session: self.active_session.clone(),
                delivery_tracked: self.pending_prompt_delivery_tracked,
                delivered: false,
            });
        self.pending_prompt_id = None;
        self.pending_prompt_delivery_unknown = false;
        self.pending_prompt_delivery_tracked = false;
        self.normal_prompt_in_flight = false;
    }

    /// A late receipt belongs to the old outbox row, never a new submission to
    /// the same target. It must not emit a composer result for that target.
    pub(super) fn reconcile_retired_prompt(&mut self, id: &str, delivered: bool) -> bool {
        let Some(retired) = self.retired_prompts.get(id).cloned() else {
            return false;
        };
        let result = (|| {
            if let Some(store) = self.state.as_mut() {
                if let Some(outbox_id) = retired.outbox_id {
                    store.complete_prompt_with_receipt(
                        outbox_id,
                        &retired.target,
                        retired.session.as_deref(),
                        id,
                        retired.delivery_tracked,
                    )?;
                }
                if delivered && !retired.delivered {
                    store.record_prompt_receipt_delivered(id, None)?;
                }
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            zlog::error!("Save late prompt receipt: {error}");
            return true;
        }
        if self.active_session == retired.session && !retired.delivered {
            conversation_mut(self.active_snapshot_mut()).record_prompt_delivery(
                id,
                &json!({"queued":true}),
                if delivered { "delivered" } else { "accepted" },
            );
            if self.parked_snapshot.is_none() {
                self.publish();
            }
        }
        if let Some(retired) = self.retired_prompts.get_mut(id) {
            retired.outbox_id = None;
            retired.delivered |= delivered;
        }
        true
    }
}
