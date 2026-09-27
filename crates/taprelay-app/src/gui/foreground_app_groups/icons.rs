use crate::platform::foreground_apps::IconPixels;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc,
};
use taprelay_core::foreground_app::executable_identity;

struct Entry {
    image: Option<Image>,
    name: Option<String>,
    missing: bool,
    fresh: bool,
    used: u64,
}

#[derive(Default)]
struct Metadata {
    icon: Option<IconPixels>,
    name: Option<String>,
    missing: bool,
}

pub(crate) struct IconCache {
    images: BTreeMap<String, Entry>,
    pending: BTreeSet<String>,
    requests: mpsc::SyncSender<String>,
    results: mpsc::Receiver<(String, Metadata)>,
    clock: u64,
}

impl IconCache {
    pub fn new() -> Self {
        Self::with_loader(|path| Metadata {
            icon: crate::platform::foreground_apps::executable_icon(path),
            name: crate::platform::foreground_apps::executable_name(path),
            // File status may block on a disconnected share; keep it off the UI thread.
            missing: !std::path::Path::new(path).is_file(),
        })
    }

    fn with_loader(loader: impl Fn(&str) -> Metadata + Send + 'static) -> Self {
        let (requests, rx) = mpsc::sync_channel::<String>(64);
        let (tx, results) = mpsc::sync_channel(64);
        let _ = std::thread::Builder::new()
            .name("taprelay-icons".into())
            .spawn(move || {
                while let Ok(path) = rx.recv() {
                    let icon = loader(&path);
                    if tx.send((path, icon)).is_err() {
                        break;
                    }
                }
            });
        Self {
            images: BTreeMap::new(),
            pending: BTreeSet::new(),
            requests,
            results,
            clock: 0,
        }
    }

    pub fn request(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let key = executable_identity(path);
        self.clock += 1;
        if let Some(entry) = self.images.get_mut(&key) {
            entry.used = self.clock;
            if entry.fresh {
                return;
            }
        }
        if self.pending.contains(&key) {
            return;
        }
        if self.requests.try_send(path.to_string()).is_ok() {
            self.pending.insert(key);
        }
    }

    pub fn image(&self, path: &str) -> Option<Image> {
        self.images
            .get(&executable_identity(path))
            .and_then(|entry| entry.image.clone())
    }

    /// Reopening the editor rechecks visible paths without discarding their
    /// current icons while a disconnected share or slow drive is being probed.
    pub fn refresh(&mut self) {
        for entry in self.images.values_mut() {
            entry.fresh = false;
        }
    }

    pub fn name(&self, path: &str) -> Option<&str> {
        self.images
            .get(&executable_identity(path))
            .and_then(|entry| entry.name.as_deref())
    }

    pub fn missing(&self, path: &str) -> bool {
        self.images
            .get(&executable_identity(path))
            .is_some_and(|entry| entry.missing)
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((path, metadata)) = self.results.try_recv() {
            let key = executable_identity(&path);
            self.pending.remove(&key);
            let image = metadata.icon.map(|pixels| {
                let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(pixels.size, pixels.size);
                buffer.make_mut_bytes().copy_from_slice(&pixels.rgba);
                Image::from_rgba8(buffer)
            });
            if self.images.len() >= 512 && !self.images.contains_key(&key) {
                let oldest = self
                    .images
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| key.clone())
                    .unwrap();
                self.images.remove(&oldest);
            }
            self.clock += 1;
            self.images.insert(
                key,
                Entry {
                    image,
                    name: metadata.name,
                    missing: metadata.missing,
                    fresh: true,
                    used: self.clock,
                },
            );
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_pixels_are_published_as_a_cached_image() {
        let mut cache = IconCache::with_loader(|_| Metadata {
            icon: Some(IconPixels {
                size: 1,
                rgba: vec![80, 120, 200, 255],
            }),
            name: Some("Friendly app name".into()),
            missing: false,
        });
        cache.request(r"C:\Apps\one.exe");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !cache.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(cache.image("c:/apps/ONE.exe").is_some());
        assert_eq!(cache.name("c:/apps/ONE.exe"), Some("Friendly app name"));
        assert!(!cache.poll());
    }
    #[test]
    fn lazy_requests_deduplicate_paths_and_cache_failures() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let mut cache = IconCache::with_loader(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Metadata {
                name: Some("Name without an icon".into()),
                ..Metadata::default()
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        cache.request(r"C:\Apps\one.exe");
        cache.request("c:/apps/ONE.exe");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !cache.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        cache.request(r"C:\Apps\one.exe");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cache.image(r"C:\Apps\one.exe").is_none());
        assert_eq!(cache.name(r"C:\Apps\one.exe"), Some("Name without an icon"));
        assert!(cache.pending.is_empty());
    }

    #[test]
    fn missing_names_stay_optional_for_the_filename_fallback() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut cache = IconCache::with_loader(move |_| Metadata {
            missing: calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0,
            ..Metadata::default()
        });
        assert!(!cache.missing(r"C:\Apps\unnamed.exe"));
        cache.request(r"C:\Apps\unnamed.exe");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !cache.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(cache.name(r"C:\Apps\unnamed.exe").is_none());
        assert!(cache.missing("c:/apps/UNNAMED.exe"));
        cache.refresh();
        cache.request(r"C:\Apps\unnamed.exe");
        while !cache.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!cache.missing(r"C:\Apps\unnamed.exe"));
    }
}
