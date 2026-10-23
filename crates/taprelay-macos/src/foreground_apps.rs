//! macOS application identity is a bundle identifier, or an explicitly tagged
//! canonical executable path for processes with no bundle.
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSArray, NSBundle, NSString, NSURL};
use std::path::Path;
use taprelay_core::foreground_app::valid_executable;

unsafe fn identity(app: &AnyObject) -> Option<String> {
    unsafe {
        let bundle: Option<Retained<NSString>> = msg_send![app, bundleIdentifier];
        if let Some(bundle) = bundle {
            let id = format!("bundle:{bundle}");
            if valid_executable(&id) {
                return Some(id);
            }
        }
        let url: Option<Retained<NSURL>> = msg_send![app, executableURL];
        let path = url.and_then(|url| url.path())?;
        let path = std::fs::canonicalize(path.to_string()).ok()?;
        let id = format!("mac-path:{}", path.to_string_lossy());
        valid_executable(&id).then_some(id)
    }
}
fn workspace() -> Retained<AnyObject> {
    unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] }
}
#[derive(Default)]
pub struct Foreground {
    path: Option<String>,
    pid: Option<i32>,
}
impl Foreground {
    pub fn path(&mut self) -> Option<&str> {
        unsafe {
            let app: Option<Retained<AnyObject>> = msg_send![&*workspace(), frontmostApplication];
            let pid = app.as_ref().map(|app| msg_send![&**app, processIdentifier]);
            if self.pid != pid || self.path.is_none() {
                self.path = app.and_then(|app| identity(&app));
                self.pid = pid;
            }
            self.path.as_deref()
        }
    }

    pub(super) fn pid(&self) -> Option<i32> {
        self.pid
    }
}
pub fn running() -> std::io::Result<Vec<String>> {
    objc2::rc::autoreleasepool(|_| running_inner())
}
fn running_inner() -> std::io::Result<Vec<String>> {
    unsafe {
        let apps: Retained<NSArray<AnyObject>> = msg_send![&*workspace(), runningApplications];
        let mut paths = apps
            .iter()
            .filter_map(|app| {
                let policy: isize = msg_send![&*app, activationPolicy];
                (policy == 0).then(|| identity(&app)).flatten()
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
pub fn identity_for_selection(value: &str) -> std::io::Result<String> {
    objc2::rc::autoreleasepool(|_| identity_for_selection_inner(value))
}
fn identity_for_selection_inner(value: &str) -> std::io::Result<String> {
    if valid_executable(value) {
        return Ok(value.to_owned());
    }
    let path = std::fs::canonicalize(value)?;
    if path.extension().is_some_and(|ext| ext == "app") {
        if let Some(bundle) = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
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
}
fn app_path(id: &str) -> Option<String> {
    if let Some(path) = id.strip_prefix("mac-path:") {
        return Some(path.into());
    }
    let bundle = id.strip_prefix("bundle:")?;
    unsafe {
        let url: Option<Retained<NSURL>> = msg_send![&*workspace(), URLForApplicationWithBundleIdentifier:&*NSString::from_str(bundle)];
        url.and_then(|u| u.path()).map(|p| p.to_string())
    }
}
pub fn executable_name(id: &str) -> Option<String> {
    objc2::rc::autoreleasepool(|_| executable_name_inner(id))
}
fn executable_name_inner(id: &str) -> Option<String> {
    let path = app_path(id)?;
    if let Some(bundle) = NSBundle::bundleWithPath(&NSString::from_str(&path)) {
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            if let Some(name) = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))
                && let Ok(name) = name.downcast::<NSString>()
            {
                return Some(name.to_string());
            }
        }
    }
    Path::new(&path)
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
}
pub struct IconPixels {
    pub size: u32,
    pub rgba: Vec<u8>,
}
pub fn executable_icon(id: &str) -> Option<IconPixels> {
    objc2::rc::autoreleasepool(|_| executable_icon_inner(id))
}
fn executable_icon_inner(id: &str) -> Option<IconPixels> {
    unsafe {
        use objc2_foundation::NSDictionary;
        let path = app_path(id)?;
        let icon: Retained<AnyObject> =
            msg_send![&*workspace(), iconForFile:&*NSString::from_str(&path)];
        let tiff: Option<Retained<objc2_foundation::NSData>> =
            msg_send![&*icon, TIFFRepresentation];
        let bitmap: Option<Retained<AnyObject>> =
            msg_send![class!(NSBitmapImageRep), imageRepWithData:&**tiff.as_ref()?];
        let properties = NSDictionary::<NSString, AnyObject>::new();
        let png: Option<Retained<objc2_foundation::NSData>> =
            msg_send![&**bitmap.as_ref()?, representationUsingType:4usize, properties:&*properties];
        let image = image::load_from_memory(&png?.to_vec())
            .ok()?
            .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        Some(IconPixels {
            size: 32,
            rgba: image.into_raw(),
        })
    }
}

pub fn exists(id: &str) -> bool {
    objc2::rc::autoreleasepool(|_| exists_inner(id))
}
fn exists_inner(id: &str) -> bool {
    app_path(id).is_some_and(|path| Path::new(&path).exists())
}
