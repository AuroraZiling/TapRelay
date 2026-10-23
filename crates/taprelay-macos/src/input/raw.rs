//! Physical relative motion owns the mouse stream. Quartz owns keyboard,
//! button and wheel edges and suppresses local input without forwarding a
//! second copy of mouse motion. Both sources live on the same run loop.
use super::*;
use core_foundation::{
    array::CFArray, base::TCFType, dictionary::CFDictionary, number::CFNumber, string::CFString,
};

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}
unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> *mut c_void;
    fn IOHIDManagerSetDeviceMatchingMultiple(manager: *mut c_void, matches: *const c_void);
    fn IOHIDManagerOpen(manager: *mut c_void, options: u32) -> i32;
    fn IOHIDManagerClose(manager: *mut c_void, options: u32) -> i32;
    fn IOHIDManagerScheduleWithRunLoop(
        manager: *mut c_void,
        run_loop: *mut c_void,
        mode: *const c_void,
    );
    fn IOHIDManagerUnscheduleFromRunLoop(
        manager: *mut c_void,
        run_loop: *mut c_void,
        mode: *const c_void,
    );
    fn IOHIDManagerRegisterInputValueCallback(
        manager: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void),
        context: *mut c_void,
    );
    fn IOHIDManagerRegisterDeviceRemovalCallback(
        manager: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void),
        context: *mut c_void,
    );
    fn IOHIDManagerRegisterDeviceMatchingCallback(
        manager: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, *mut c_void),
        context: *mut c_void,
    );
    fn IOHIDDeviceConformsTo(device: *mut c_void, page: u32, usage: u32) -> u8;
    fn IOHIDDeviceCopyMatchingElements(
        device: *mut c_void,
        matches: *const c_void,
        options: u32,
    ) -> *mut c_void;
    fn IOHIDValueGetElement(value: *mut c_void) -> *mut c_void;
    fn IOHIDValueGetIntegerValue(value: *mut c_void) -> isize;
    fn IOHIDValueGetLength(value: *mut c_void) -> isize;
    fn IOHIDValueGetTimeStamp(value: *mut c_void) -> u64;
    fn IOHIDElementGetUsagePage(element: *mut c_void) -> u32;
    fn IOHIDElementGetUsage(element: *mut c_void) -> u32;
    fn IOHIDElementGetDevice(element: *mut c_void) -> *mut c_void;
    fn IOHIDElementGetType(element: *mut c_void) -> u32;
    fn IOHIDElementIsRelative(element: *mut c_void) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *mut c_void;
}

#[derive(Clone, Copy, Default)]
struct Device {
    keyboard: bool,
    relative_mouse: bool,
}

#[derive(Default)]
pub(super) struct Devices(std::collections::BTreeMap<usize, Device>);

impl Devices {
    pub fn counts(&self) -> (usize, usize) {
        (
            self.0.values().filter(|d| d.keyboard).count(),
            self.0.values().filter(|d| d.relative_mouse).count(),
        )
    }
    pub fn error(&self) -> Option<&'static str> {
        if !self.0.values().any(|device| device.keyboard) {
            Some(MISSING_PHYSICAL_KEYBOARD)
        } else if !self.0.values().any(|device| device.relative_mouse) {
            Some(MISSING_RELATIVE_MOUSE)
        } else {
            None
        }
    }
}

pub(super) struct Capture(*mut c_void);
impl Capture {
    // Construct and drop on the event thread that owns this run-loop source.
    pub(super) unsafe fn new(context: *mut c_void) -> Result<Self, String> {
        unsafe {
            let manager = IOHIDManagerCreate(ptr::null(), 0);
            if manager.is_null() {
                return Err("Cannot create physical HID observer".into());
            }
            let matches = CFArray::from_CFTypes(&[6, 2].map(|usage| {
                CFDictionary::from_CFType_pairs(&[
                    (CFString::new("DeviceUsagePage"), CFNumber::from(1)),
                    (CFString::new("DeviceUsage"), CFNumber::from(usage)),
                ])
            }));
            IOHIDManagerSetDeviceMatchingMultiple(manager, matches.as_concrete_TypeRef().cast());
            IOHIDManagerRegisterInputValueCallback(manager, value_changed, context);
            IOHIDManagerRegisterDeviceMatchingCallback(manager, device_added, context);
            IOHIDManagerRegisterDeviceRemovalCallback(manager, device_removed, context);
            IOHIDManagerScheduleWithRunLoop(
                manager,
                CFRunLoopGetCurrent(),
                kCFRunLoopDefaultMode.cast(),
            );
            let result = IOHIDManagerOpen(manager, 0);
            let capture = Self(manager);
            if result != 0 {
                return Err(format!(
                    "Cannot observe physical keyboard/mouse ({result:#x}); check Input Monitoring"
                ));
            }
            Ok(capture)
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            IOHIDManagerClose(self.0, 0);
            IOHIDManagerUnscheduleFromRunLoop(
                self.0,
                CFRunLoopGetCurrent(),
                kCFRunLoopDefaultMode.cast(),
            );
            CFRelease(self.0);
        }
    }
}

fn relative_motion(page: u32, usage: u32, value: i32, relative: bool) -> Option<[i32; 2]> {
    if page != 1 || !relative {
        return None;
    }
    match usage {
        0x30 => Some([value, 0]), // Generic Desktop X
        0x31 => Some([0, value]), // Generic Desktop Y
        _ => None,
    }
}

fn timestamp_nanos(timestamp: u64) -> u64 {
    static TIMEBASE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let &(numer, denom) = TIMEBASE.get_or_init(|| unsafe {
        let mut info = Timebase { numer: 0, denom: 0 };
        assert_eq!(mach_timebase_info(&mut info), 0, "HID clock unavailable");
        assert_ne!(info.denom, 0, "HID clock timebase unavailable");
        (info.numer, info.denom)
    });
    (u128::from(timestamp) * u128::from(numer) / u128::from(denom)).min(u128::from(u64::MAX)) as u64
}
fn captured_at(timestamp: u64) -> Instant {
    let nanos =
        timestamp_nanos(unsafe { mach_absolute_time() }).saturating_sub(timestamp_nanos(timestamp));
    let now = Instant::now();
    now.checked_sub(Duration::from_nanos(nanos)).unwrap_or(now)
}

unsafe extern "C" fn value_changed(
    context: *mut c_void,
    result: i32,
    _sender: *mut c_void,
    value: *mut c_void,
) {
    let shared = unsafe { &*(context as *const Shared) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let Ok(mut policy) = shared.policy.try_lock() else {
            shared.stop_after_callback(CallbackFailure::RawReentered);
            return;
        };
        if result != 0 {
            policy.error = Some(format!("Physical HID observation failed ({result:#x})"));
            policy.end_capture();
            return;
        }
        let element = IOHIDValueGetElement(value);
        if !(1..=4).contains(&IOHIDElementGetType(element))
            || IOHIDValueGetLength(value) > std::mem::size_of::<isize>() as isize
        {
            return;
        }
        let page = IOHIDElementGetUsagePage(element);
        let usage = IOHIDElementGetUsage(element);
        let relative = IOHIDElementIsRelative(element) != 0;
        if policy.router.passthrough() && page == 1 && matches!(usage, 0x30 | 0x31) && !relative {
            policy.fail_capture("Absolute mouse input is not supported by passthrough");
            return;
        }
        let Some(delta) = relative_motion(
            page,
            usage,
            IOHIDValueGetIntegerValue(value).clamp(i32::MIN as isize, i32::MAX as isize) as i32,
            relative,
        ) else {
            return;
        };
        let captured = captured_at(IOHIDValueGetTimeStamp(value));
        if !policy.router.passthrough()
            || policy
                .capture
                .started()
                .is_none_or(|start| captured < start)
        {
            return;
        }
        if !SESSION_ACTIVE.load(Ordering::Acquire) || IsSecureEventInputEnabled() {
            policy.error =
                Some("Physical capture revoked by session change or Secure Input".into());
            policy.end_capture();
            return;
        }
        let device = IOHIDElementGetDevice(element) as usize;
        if !policy
            .devices
            .0
            .get(&device)
            .is_some_and(|device| device.relative_mouse)
        {
            return;
        }
        if let Err(error) = policy.motion.push(
            device,
            timestamp_nanos(IOHIDValueGetTimeStamp(value)),
            captured,
            delta,
        ) {
            policy.error = Some(error.into());
            policy.end_capture();
        }
    }))
    .is_err()
    {
        shared.stop_after_callback(CallbackFailure::RawPanicked);
    }
}

unsafe extern "C" fn device_added(
    context: *mut c_void,
    result: i32,
    _sender: *mut c_void,
    device: *mut c_void,
) {
    let shared = unsafe { &*(context as *const Shared) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let Ok(mut policy) = shared.policy.try_lock() else {
            shared.stop_after_callback(CallbackFailure::RawReentered);
            return;
        };
        if result != 0 {
            policy.error = Some(format!("Physical HID device unavailable ({result:#x})"));
            policy.end_capture();
            return;
        }
        let keyboard = IOHIDDeviceConformsTo(device, 1, 6) != 0;
        let mut axes = [false; 2];
        if IOHIDDeviceConformsTo(device, 1, 2) != 0 {
            let elements = IOHIDDeviceCopyMatchingElements(device, ptr::null(), 0);
            if !elements.is_null() {
                for index in 0..CFArrayGetCount(elements) {
                    let element = CFArrayGetValueAtIndex(elements, index);
                    if (1..=4).contains(&IOHIDElementGetType(element))
                        && IOHIDElementGetUsagePage(element) == 1
                        && IOHIDElementIsRelative(element) != 0
                    {
                        match IOHIDElementGetUsage(element) {
                            0x30 => axes[0] = true,
                            0x31 => axes[1] = true,
                            _ => {}
                        }
                    }
                }
                CFRelease(elements);
            }
        }
        policy.devices.0.insert(
            device as usize,
            Device {
                keyboard,
                relative_mouse: axes == [true, true],
            },
        );
        tracing::debug!(
            keyboard,
            relative_mouse = axes == [true, true],
            "Physical input device observed"
        );
    }))
    .is_err()
    {
        shared.stop_after_callback(CallbackFailure::RawPanicked);
    }
}

unsafe extern "C" fn device_removed(
    context: *mut c_void,
    _result: i32,
    _sender: *mut c_void,
    device: *mut c_void,
) {
    let shared = unsafe { &*(context as *const Shared) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Ok(mut policy) = shared.policy.try_lock() {
            let removed = policy.devices.0.remove(&(device as usize));
            if removed.is_some_and(|d| d.keyboard || d.relative_mouse)
                && (policy.router.passthrough()
                    || policy
                        .link
                        .as_ref()
                        .is_some_and(InputLink::capture_requested))
            {
                policy.fail_capture("Physical keyboard or mouse disconnected; capture stopped");
            }
        } else {
            shared.stop_after_callback(CallbackFailure::RawReentered);
        }
    }))
    .is_err()
    {
        shared.stop_after_callback(CallbackFailure::RawPanicked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_requires_keyboard_and_relative_mouse_but_allows_composite_devices() {
        let mut devices = Devices::default();
        assert!(devices.error().is_some());
        devices.0.insert(
            1,
            Device {
                keyboard: true,
                relative_mouse: false,
            },
        );
        assert!(devices.error().is_some());
        devices.0.insert(
            2,
            Device {
                keyboard: false,
                relative_mouse: true,
            },
        );
        assert!(devices.error().is_none());
        devices.0.remove(&1);
        assert!(devices.error().is_some());
        devices.0.insert(
            2,
            Device {
                keyboard: true,
                relative_mouse: true,
            },
        );
        assert!(devices.error().is_none());
    }
    #[test]
    fn raw_observation_only_supplies_relative_motion() {
        assert_eq!(relative_motion(1, 0x30, 1200, true), Some([1200, 0]));
        assert_eq!(relative_motion(1, 0x31, -3, true), Some([0, -3]));
        assert_eq!(relative_motion(1, 0x30, 1200, false), None);
        for (page, usage) in [(7, 4), (7, 0xe1), (9, 1), (1, 0x38), (0x0c, 0x238)] {
            for value in [1, 0] {
                assert_eq!(
                    relative_motion(page, usage, value, true),
                    None,
                    "raw edges must never duplicate the Quartz route"
                );
            }
        }
    }
}
