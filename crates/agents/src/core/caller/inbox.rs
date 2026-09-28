use super::*;
use crate::SessionInbox;

pub(super) struct Mailbox {
    pub(super) receiver: mpsc::Receiver<PeerMessage>,
    pub(super) pending: Option<PeerMessage>,
}

impl Mailbox {
    pub(super) fn has_pending(&mut self) -> bool {
        if self.pending.is_none() {
            self.pending = self.receiver.try_recv().ok();
        }
        self.pending.is_some()
    }
}

pub(super) struct RetainedInboxState {
    route: Mutex<RetainedRoute>,
    transferred: AtomicBool,
}

struct RetainedRoute {
    token: String,
    mailbox: Arc<Mutex<Mailbox>>,
}

impl RetainedInboxState {
    pub(super) fn detached(&self) {
        self.transferred.store(false, Ordering::Release);
    }
}

struct RetainedInbox {
    registry: CallerRegistry,
    state: Arc<RetainedInboxState>,
}

impl SessionInbox for RetainedInbox {
    fn has_pending_messages(&self) -> bool {
        self.state.route.lock().map_or(true, |route| {
            route
                .mailbox
                .lock()
                .map_or(true, |mut inbox| inbox.has_pending())
        })
    }

    fn transferred(&self) -> bool {
        self.state.transferred.load(Ordering::Acquire)
    }
}

impl Drop for RetainedInbox {
    fn drop(&mut self) {
        if let Ok(mut callers) = self.registry.callers.lock()
            && let Ok(route) = self.state.route.lock()
        {
            if let Some(caller) = callers.get_mut(&route.token) {
                caller.retained_inbox = Weak::new();
            }
            if let Ok(mut retired) = self.registry.retired_inboxes.lock() {
                retired.remove(&route.token);
            }
        }
    }
}

impl CallerIdentity {
    pub fn retain_inbox(&self) -> Result<Option<Box<dyn SessionInbox>>, String> {
        let mut callers = self
            .registry
            .callers
            .lock()
            .map_err(|_| "caller registry unavailable")?;
        let caller = callers
            .get_mut(&self.token)
            .ok_or("caller is not registered")?;
        if caller.session_key().is_none() {
            return Err("cannot retain an unbound session inbox".into());
        }
        if caller.retained_inbox.upgrade().is_some() {
            return Err("session inbox is already retained".into());
        }
        let state = Arc::new(RetainedInboxState {
            route: Mutex::new(RetainedRoute {
                token: self.token.clone(),
                mailbox: self.inbox.clone(),
            }),
            transferred: AtomicBool::new(false),
        });
        caller.retained_inbox = Arc::downgrade(&state);
        Ok(Some(Box::new(RetainedInbox {
            registry: self.registry.clone(),
            state,
        })))
    }
}

impl CallerRegistry {
    pub(super) fn adopt_retained_inbox(&self, token: &str) {
        let Ok(mut callers) = self.callers.lock() else {
            return;
        };
        let Some(caller) = callers.get_mut(token) else {
            return;
        };
        let Some(session) = caller.session_key() else {
            return;
        };
        let Ok(mut retired) = self.retired_inboxes.lock() else {
            return;
        };
        let mut candidates = retired.iter().filter(|(_, old)| {
            old.harness_profile_id == caller.harness_profile_id
                && old
                    .session_key()
                    .is_some_and(|old| old.same_session(&session))
        });
        let Some((old_token, old)) = candidates.next() else {
            return;
        };
        if candidates.next().is_some() {
            return;
        }
        let old_token = old_token.clone();
        let Some(state) = old.retained_inbox.upgrade() else {
            return;
        };
        let Ok(mut route) = state.route.lock() else {
            return;
        };
        let old_mailbox = route.mailbox.clone();
        let Ok(mut mailbox) = old_mailbox.lock() else {
            return;
        };
        while mailbox.has_pending() {
            let message = mailbox.pending.take().expect("checked pending message");
            if let Err(error) = caller.inbox.send(message) {
                mailbox.pending = Some(error.0);
                return;
            }
        }
        drop(mailbox);
        route.token = token.to_owned();
        route.mailbox = caller.mailbox.clone();
        caller.retained_inbox = Arc::downgrade(&state);
        state.transferred.store(true, Ordering::Release);
        retired.remove(&old_token);
    }
}
