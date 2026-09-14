use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use gpui::Context;

use super::FarcasterApp;

const RECENT_COMPLETION_LIFETIME: Duration = Duration::from_secs(10 * 60);

impl FarcasterApp {
    pub(in crate::app) fn sync_notification_expiries(&mut self, cx: &mut Context<Self>) {
        let pending = self
            .extensions
            .active
            .notifications
            .iter()
            .chain(
                self.extensions
                    .parked
                    .iter()
                    .flat_map(|extension| extension.notifications.iter()),
            )
            .map(|notification| (notification.id.clone(), notification.expires_at))
            .collect::<HashSet<_>>();
        self.extensions
            .notification_expiries
            .retain(|notification, _| pending.contains(notification));
        for (id, expires_at) in pending {
            let notification = (id.clone(), expires_at);
            if self
                .extensions
                .notification_expiries
                .contains_key(&notification)
            {
                continue;
            }
            let wait = expires_at.saturating_duration_since(Instant::now());
            let task = cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(wait).await;
                let _ = weak.update(cx, |this, cx| {
                    let removed = this.extensions.active.remove_notification(&id, expires_at)
                        | this.extensions.parked.as_mut().is_some_and(|extension| {
                            extension.remove_notification(&id, expires_at)
                        });
                    if removed {
                        this.extensions
                            .notification_expiries
                            .remove(&(id, expires_at));
                        cx.notify();
                    }
                });
            });
            self.extensions
                .notification_expiries
                .insert(notification, task);
        }
    }

    pub(in crate::app) fn sync_recent_completion_expiries(&mut self, cx: &mut Context<Self>) {
        self.activity
            .recent_completion_expiries
            .retain(|target, (completed_at, _)| {
                self.activity.recent_completions.get(target) == Some(completed_at)
            });
        let pending = self
            .activity
            .recent_completions
            .iter()
            .filter(|(target, completed_at)| {
                self.activity
                    .recent_completion_expiries
                    .get(*target)
                    .is_none_or(|(scheduled_at, _)| scheduled_at != *completed_at)
            })
            .map(|(target, completed_at)| (target.clone(), *completed_at))
            .collect::<Vec<_>>();
        for (target, completed_at) in pending {
            let wait = (completed_at + RECENT_COMPLETION_LIFETIME)
                .saturating_duration_since(Instant::now());
            let task_target = target.clone();
            let task = cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(wait).await;
                let _ = weak.update(cx, |this, cx| {
                    if expire_recent_completion(
                        &mut this.activity.recent_completions,
                        &mut this.activity.run_statuses,
                        &task_target,
                        completed_at,
                    ) {
                        this.activity
                            .recent_completion_expiries
                            .remove(&task_target);
                        this.notify_session_rail(cx);
                    }
                });
            });
            self.activity
                .recent_completion_expiries
                .insert(target, (completed_at, task));
        }
    }
}

fn expire_recent_completion(
    recent_completions: &mut HashMap<String, Instant>,
    run_statuses: &mut HashMap<String, String>,
    target: &str,
    completed_at: Instant,
) -> bool {
    if recent_completions.get(target) != Some(&completed_at) {
        return false;
    }
    recent_completions.remove(target);
    if run_statuses
        .get(target)
        .is_some_and(|status| status == "Done")
    {
        run_statuses.remove(target);
    }
    true
}

#[cfg(test)]
#[path = "expiries_tests.rs"]
mod tests;
