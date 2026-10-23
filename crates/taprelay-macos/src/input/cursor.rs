//! Quartz cursor control requires the application to be foreground. Acquire
//! that ownership before swallowing input, and restore it on every exit path.
use super::{CGEventCreate, CGEventGetLocation, Point};
use core_foundation::base::CFRelease;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use std::{
    ptr,
    time::{Duration, Instant},
};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGWarpMouseCursorPosition(position: Point) -> i32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
}

pub(super) struct Capture {
    previous: Option<Retained<NSRunningApplication>>,
    current: Retained<NSRunningApplication>,
    position: Point,
    requested: Instant,
    hidden: bool,
    detached: bool,
}

impl Capture {
    // Called only by the input run loop, never from the event-tap callback.
    pub(super) fn request(now: Instant) -> Result<Self, &'static str> {
        let event = unsafe { CGEventCreate(ptr::null()) };
        if event.is_null() {
            return Err("Cannot save the local cursor position");
        }
        let position = unsafe { CGEventGetLocation(event) };
        unsafe { CFRelease(event) };
        let current = NSRunningApplication::currentApplication();
        let previous = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .filter(|app| app.processIdentifier() != current.processIdentifier());
        let capture = Self {
            previous,
            current,
            position,
            requested: now,
            hidden: false,
            detached: false,
        };
        if capture.previous.is_some() && !activate(&capture.current) {
            return Err("macOS did not accept the passthrough activation request");
        }
        Ok(capture)
    }
}

impl super::capture::CursorLease for Capture {
    fn ready(&mut self, foreground_is_ours: bool, now: Instant) -> Result<bool, &'static str> {
        if !foreground_is_ours {
            if self.detached
                || now.saturating_duration_since(self.requested) >= Duration::from_secs(1)
            {
                return Err("TapRelay lost foreground cursor control; capture stopped");
            }
            return Ok(false);
        }
        if !self.hidden {
            if unsafe { CGDisplayHideCursor(0) } != 0 {
                return Err("macOS could not hide the local cursor; capture stopped");
            }
            self.hidden = true;
        }
        if !self.detached {
            if unsafe { CGAssociateMouseAndMouseCursorPosition(false) } != 0 {
                return Err("macOS could not detach the local cursor; capture stopped");
            }
            self.detached = true;
            tracing::info!("Foreground cursor control acquired");
        }
        Ok(true)
    }
}

fn activate(app: &NSRunningApplication) -> bool {
    // The flag is needed on the supported pre-14 systems; later systems ignore it.
    #[allow(deprecated)]
    app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps)
}

impl Drop for Capture {
    fn drop(&mut self) {
        let ours = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|app| app.processIdentifier() == self.current.processIdentifier());
        unsafe {
            if self.detached {
                let status = CGAssociateMouseAndMouseCursorPosition(true);
                if status != 0 {
                    tracing::warn!(status, "Could not reattach cursor after capture");
                }
            }
            if ours && (self.hidden || self.detached) {
                let status = CGWarpMouseCursorPosition(self.position);
                if status != 0 {
                    tracing::warn!(status, "Could not restore saved cursor position");
                }
            }
            if self.hidden {
                let status = CGDisplayShowCursor(0);
                if status != 0 {
                    tracing::warn!(status, "Could not show cursor after capture");
                }
            }
        }
        // Do not steal focus from an app the user/system selected during
        // capture, or try to activate an app over the lock screen.
        if ours
            && super::SESSION_ACTIVE.load(std::sync::atomic::Ordering::Acquire)
            && let Some(previous) = &self.previous
            && !previous.isTerminated()
        {
            let accepted = activate(previous);
            tracing::info!(accepted, "Requested previous application restoration");
        }
    }
}
