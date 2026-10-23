use std::path::PathBuf;

use objc2::rc::autoreleasepool;
use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};

pub fn application_support() -> std::io::Result<PathBuf> {
    Ok(user_directory(NSSearchPathDirectory::ApplicationSupportDirectory)?.join("TapRelay"))
}

pub fn logs() -> std::io::Result<PathBuf> {
    // Foundation has no Logs directory constant; resolve the user Library first.
    Ok(user_directory(NSSearchPathDirectory::LibraryDirectory)?.join("Logs/TapRelay"))
}

fn user_directory(directory: NSSearchPathDirectory) -> std::io::Result<PathBuf> {
    autoreleasepool(|_| {
        NSFileManager::defaultManager()
            .URLsForDirectory_inDomains(directory, NSSearchPathDomainMask::UserDomainMask)
            .firstObject()
            .and_then(|url| url.to_file_path())
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                std::io::Error::other(format!("Cannot resolve macOS user directory {directory:?}"))
            })
    })
}
