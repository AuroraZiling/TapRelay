//! macOS application identity is a bundle identifier, or an explicitly tagged
//! canonical executable path for processes with no bundle.
use objc2::{AnyThread, rc::Retained};
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSBitmapFormat, NSBitmapImageRep, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSGraphicsContext, NSImage, NSImageInterpolation, NSRunningApplication,
    NSWorkspace,
};
use objc2_foundation::{NSBundle, NSCopying, NSPoint, NSRect, NSSize, NSString};
use std::path::{Path, PathBuf};
use taprelay_core::foreground_app::valid_executable;

fn identity(app: &NSRunningApplication) -> Option<String> {
    if let Some(bundle) = app.bundleIdentifier() {
        let id = format!("bundle:{bundle}");
        if valid_executable(&id) {
            return Some(id);
        }
    }
    let path = app.executableURL()?.path()?;
    let path = std::fs::canonicalize(path.to_string()).ok()?;
    let id = format!("mac-path:{}", path.to_string_lossy());
    valid_executable(&id).then_some(id)
}

#[derive(Default)]
pub struct Foreground {
    path: Option<String>,
    pid: Option<i32>,
}
impl Foreground {
    pub fn path(&mut self) -> Option<&str> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication();
        let pid = app.as_ref().map(|app| app.processIdentifier());
        if self.pid != pid || self.path.is_none() {
            self.path = app.and_then(|app| identity(&app));
            self.pid = pid;
        }
        self.path.as_deref()
    }

    pub(super) fn pid(&self) -> Option<i32> {
        self.pid
    }
}

/// Cheap discovery data: keep the running instance's URL instead of asking
/// LaunchServices to pick an installation again when its icon is loaded.
#[derive(Clone, Debug)]
pub struct RunningApp {
    pub identity: String,
    pub name: Option<String>,
    pub path: Option<PathBuf>,
}

pub fn running() -> std::io::Result<Vec<RunningApp>> {
    objc2::rc::autoreleasepool(|_| {
        let mut apps = NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
            .filter_map(|app| {
                Some(RunningApp {
                    identity: identity(&app)?,
                    name: app
                        .localizedName()
                        .and_then(|name| display_name(name.to_string())),
                    path: app
                        .bundleURL()
                        .or_else(|| app.executableURL())
                        .and_then(|url| url.path())
                        .map(|path| PathBuf::from(path.to_string())),
                })
            })
            .collect::<Vec<_>>();
        apps.sort_by(|a, b| a.identity.cmp(&b.identity));
        apps.dedup_by(|a, b| a.identity == b.identity);
        Ok(apps)
    })
}

pub fn identity_for_selection(value: &str) -> std::io::Result<String> {
    objc2::rc::autoreleasepool(|_| {
        if valid_executable(value) {
            return Ok(value.to_owned());
        }
        let path = std::fs::canonicalize(value)?;
        if path.extension().is_some_and(|ext| ext == "app") {
            if let Some(bundle) =
                NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
                && let Some(id) = bundle.bundleIdentifier()
            {
                let value = format!("bundle:{id}");
                if valid_executable(&value) {
                    return Ok(value);
                }
            }
            return Err(std::io::Error::other(
                "Application has no valid bundle identifier",
            ));
        }
        let value = format!("mac-path:{}", path.to_string_lossy());
        if valid_executable(&value) {
            Ok(value)
        } else {
            Err(std::io::Error::other("Invalid application identity"))
        }
    })
}

fn app_path(id: &str) -> Option<PathBuf> {
    if let Some(path) = id.strip_prefix("mac-path:") {
        return Some(path.into());
    }
    NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(id.strip_prefix("bundle:")?))
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
}

fn display_name(name: String) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && !name.chars().any(char::is_control)).then(|| name.to_owned())
}

fn bundle_name(path: &Path) -> Option<String> {
    if let Some(bundle) = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy())) {
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            if let Some(name) = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))
                && let Ok(name) = name.downcast::<NSString>()
                && let Some(name) = display_name(name.to_string())
            {
                return Some(name);
            }
        }
    }
    path.file_stem()
        .and_then(|name| display_name(name.to_string_lossy().into_owned()))
}

pub struct Metadata {
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub missing: bool,
}

/// Resolve once on the metadata worker, then use this same path for the icon.
/// A moved/uninstalled cached application may require one new LaunchServices lookup.
pub fn metadata(id: &str, known_path: Option<&Path>, known_name: Option<&str>) -> Metadata {
    objc2::rc::autoreleasepool(|_| {
        let (path, missing) = if let Some(path) = known_path.filter(|path| path.exists()) {
            (Some(path.to_path_buf()), false)
        } else {
            let path = app_path(id);
            let missing = path.as_ref().is_none_or(|path| !path.exists());
            (path, missing)
        };
        let name = if path.as_deref() == known_path {
            known_name.and_then(|name| display_name(name.to_owned()))
        } else {
            None
        }
        .or_else(|| path.as_deref().and_then(bundle_name));
        Metadata {
            path,
            name,
            missing,
        }
    })
}

pub struct IconPixels {
    pub size: u32,
    /// Tightly packed, straight-alpha RGBA, matching the Windows adapter contract.
    pub rgba: Vec<u8>,
}

impl Metadata {
    pub fn icon(&self) -> Option<IconPixels> {
        if self.missing {
            return None;
        }
        objc2::rc::autoreleasepool(|_| {
            let icon = NSWorkspace::sharedWorkspace()
                .iconForFile(&NSString::from_str(&self.path.as_ref()?.to_string_lossy()));
            // IconServices may share NSImage instances across callers.
            icon_pixels(&icon.copy())
        })
    }
}

// Restore the worker thread's graphics context even when rendering unwinds.
struct RestoreContext(Option<Retained<NSGraphicsContext>>);
impl Drop for RestoreContext {
    fn drop(&mut self) {
        NSGraphicsContext::setCurrentContext(self.0.as_deref());
    }
}

fn icon_pixels(icon: &NSImage) -> Option<IconPixels> {
    // 64 physical pixels cover the 24-point list icon on Retina displays.
    const SIZE: usize = 64;
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), SIZE as isize, SIZE as isize,
            8, 4, true, false, NSDeviceRGBColorSpace, NSBitmapFormat::empty(), (SIZE * 4) as isize, 32,
        )?
    };
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
    let data = bitmap.bitmapData();
    let stride = usize::try_from(bitmap.bytesPerRow()).ok()?;
    if data.is_null() || stride < SIZE * 4 {
        return None;
    }
    let length = stride.checked_mul(SIZE)?;
    // The bitmap owns this allocation until the copy below is complete.
    unsafe {
        data.write_bytes(0, length);
    }
    let _restore = RestoreContext(NSGraphicsContext::currentContext());
    NSGraphicsContext::setCurrentContext(Some(&context));
    context.setImageInterpolation(NSImageInterpolation::High);
    unsafe {
        icon.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(SIZE as f64, SIZE as f64)),
            NSRect::ZERO,
            NSCompositingOperation::Copy,
            1.,
            false,
            None,
        );
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, length) };
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for row in bytes.chunks_exact(stride) {
        rgba.extend_from_slice(&row[..SIZE * 4]);
    }
    // AppKit draws premultiplied pixels; callers expect straight-alpha RGBA.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    Some(IconPixels {
        size: SIZE as u32,
        rgba,
    })
}

#[cfg(test)]
mod tests;
