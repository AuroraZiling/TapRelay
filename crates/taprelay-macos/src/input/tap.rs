//! Run-loop-owned Quartz resources. Partial construction and shutdown share
//! the same teardown path; callback context outlives this guard.
use super::*;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortInvalidate(port: *mut c_void);
}

pub(super) struct EventTap {
    port: std::ptr::NonNull<c_void>,
    source: std::ptr::NonNull<c_void>,
    run_loop: *mut c_void,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl EventTap {
    pub unsafe fn install(context: *mut c_void) -> Result<Self, &'static str> {
        unsafe {
            use CGEventType as E;
            let mask = [
                E::LeftMouseDown,
                E::LeftMouseUp,
                E::RightMouseDown,
                E::RightMouseUp,
                E::MouseMoved,
                E::LeftMouseDragged,
                E::RightMouseDragged,
                E::KeyDown,
                E::KeyUp,
                E::FlagsChanged,
                E::ScrollWheel,
                E::OtherMouseDown,
                E::OtherMouseUp,
                E::OtherMouseDragged,
            ]
            .into_iter()
            .fold(0, |mask, kind| mask | (1u64 << kind.0));
            let port = std::ptr::NonNull::new(CGEventTapCreate(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask,
                callback,
                context,
            ))
            .ok_or("CGEventTapCreate failed; check Accessibility and Input Monitoring")?;
            let Some(source) = std::ptr::NonNull::new(
                CFMachPortCreateRunLoopSource(ptr::null(), port.as_ptr(), 0).cast_mut(),
            ) else {
                CFMachPortInvalidate(port.as_ptr());
                CFRelease(port.as_ptr());
                return Err("Cannot schedule input event tap");
            };
            let guard = Self {
                port,
                source,
                run_loop: CFRunLoopGetCurrent(),
                _thread: Default::default(),
            };
            CFRunLoopAddSource(
                guard.run_loop,
                source.as_ptr(),
                kCFRunLoopDefaultMode.cast(),
            );
            CGEventTapEnable(port.as_ptr(), true);
            Ok(guard)
        }
    }
}
impl Drop for EventTap {
    fn drop(&mut self) {
        unsafe {
            CGEventTapEnable(self.port.as_ptr(), false);
            CFRunLoopRemoveSource(
                self.run_loop,
                self.source.as_ptr(),
                kCFRunLoopDefaultMode.cast(),
            );
            CFMachPortInvalidate(self.port.as_ptr());
            CFRelease(self.source.as_ptr());
            CFRelease(self.port.as_ptr());
        }
    }
}
