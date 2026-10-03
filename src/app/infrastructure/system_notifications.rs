use gpui::{App, SystemNotification};

// Headless tests use GPUI's notification recorder.
#[cfg(any(not(target_os = "macos"), test))]
pub(in crate::app) fn show(notification: SystemNotification, cx: &mut App) {
    cx.show_system_notification(notification);
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct NotificationAuthorization {
    pending: Vec<SystemNotification>,
    requesting: bool,
}

#[cfg(any(target_os = "macos", test))]
impl NotificationAuthorization {
    fn enqueue(&mut self, notification: SystemNotification) -> bool {
        // Match GPUI's replacement-by-tag behavior while permission is pending.
        self.pending.retain(|queued| queued.tag != notification.tag);
        self.pending.push(notification);
        !std::mem::replace(&mut self.requesting, true)
    }

    fn resolve(&mut self, authorized: bool) -> Vec<SystemNotification> {
        self.requesting = false;
        let pending = std::mem::take(&mut self.pending);
        if authorized { pending } else { Vec::new() }
    }
}

#[cfg(target_os = "macos")]
impl gpui::Global for NotificationAuthorization {}

#[cfg(all(target_os = "macos", not(test)))]
pub(in crate::app) fn show(notification: SystemNotification, cx: &mut App) {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};

    // The native notification center aborts for unbundled development binaries.
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return;
    }
    if !cx.has_global::<NotificationAuthorization>() {
        cx.set_global(NotificationAuthorization::default());
    }
    if !cx
        .global_mut::<NotificationAuthorization>()
        .enqueue(notification)
    {
        return;
    }

    let (sender, receiver) = async_channel::bounded(1);
    let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if !error.is_null() {
            zlog::warn!("system notification authorization failed");
        } else if !granted.as_bool() {
            zlog::info!("system notification authorization denied");
        }
        let _ = sender.try_send(error.is_null() && granted.as_bool());
    });
    // Requesting permission does not replace GPUI's notification-response delegate.
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &completion,
        );
    cx.spawn(async move |cx| {
        let authorized = receiver.recv().await.unwrap_or(false);
        let _ = cx.update(|cx| {
            for notification in cx
                .global_mut::<NotificationAuthorization>()
                .resolve(authorized)
            {
                cx.show_system_notification(notification);
            }
        });
    })
    .detach();
}

#[cfg(test)]
#[path = "system_notifications_tests.rs"]
mod tests;
