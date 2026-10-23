use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNUserNotificationCenter,
};
use std::{
    cell::Cell,
    ptr::NonNull,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use taprelay_core::ports::NotificationPermission;

#[derive(Default)]
struct Snapshot {
    permission: NotificationPermission,
    in_flight: bool,
}

pub(super) struct Notifications {
    bundled: bool,
    snapshot: Arc<Mutex<Snapshot>>,
    last_refresh: Cell<Option<Instant>>,
}

impl Notifications {
    pub fn new() -> Self {
        Self {
            // UserNotifications aborts the process for an unbundled executable.
            bundled: NSBundle::mainBundle().bundleIdentifier().is_some(),
            snapshot: Arc::default(),
            last_refresh: Cell::new(None),
        }
    }

    pub fn permission(&self) -> NotificationPermission {
        if !self.bundled {
            return NotificationPermission::Unavailable;
        }
        let mut snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        let permission = snapshot.permission;
        if snapshot.in_flight
            || self
                .last_refresh
                .get()
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return permission;
        }
        snapshot.in_flight = true;
        drop(snapshot);
        self.last_refresh.set(Some(Instant::now()));
        let state = self.snapshot.clone();
        let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // The framework owns settings for the duration of this callback.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            let mut snapshot = state.lock().unwrap_or_else(|e| e.into_inner());
            snapshot.permission = match status {
                UNAuthorizationStatus::NotDetermined => NotificationPermission::NotDetermined,
                UNAuthorizationStatus::Denied => NotificationPermission::Denied,
                UNAuthorizationStatus::Authorized => NotificationPermission::Allowed,
                UNAuthorizationStatus::Provisional => NotificationPermission::Quiet,
                _ => NotificationPermission::Unknown,
            };
            snapshot.in_flight = false;
        });
        // Reading settings does not request authorization or block the UI.
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&callback);
        permission
    }

    pub fn send(&self, title: &str, body: &str) {
        if !self.bundled {
            tracing::warn!("System notifications require an application bundle");
            return;
        }
        let title = title.to_owned();
        let body = body.to_owned();
        let callback = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                tracing::warn!(error = %error.localizedDescription(), "Notification authorization failed");
                return;
            }
            if !granted.as_bool() {
                tracing::info!("System notifications are not authorized");
                return;
            }
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&title));
            content.setBody(&NSString::from_str(&body));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str("taprelay-status"),
                &content,
                None,
            );
            let completion = RcBlock::new(|error: *mut NSError| {
                if let Some(error) = unsafe { error.as_ref() } {
                    tracing::warn!(error = %error.localizedDescription(), "System notification rejected");
                } else {
                    // Acceptance is not evidence of a visible banner (Focus,
                    // per-app alert preferences and other OS policies apply).
                    tracing::debug!("System notification accepted by macOS");
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&completion));
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert,
                &callback,
            );
    }
}
