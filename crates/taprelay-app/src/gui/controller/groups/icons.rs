use crate::platform::foreground_apps::IconPixels;
use gpui_kit::RenderImage;
use lru::LruCache;
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::mpsc,
    time::{Duration, Instant},
};
use taprelay_core::foreground_app::executable_identity;

#[derive(Default)]
struct Entry {
    image: Option<Arc<RenderImage>>,
    icon_loaded: bool,
    name: Option<String>,
    missing: bool,
    fresh: bool,
    loaded_at: Option<Instant>,
    source: Option<std::path::PathBuf>,
}

#[derive(Default)]
struct Metadata {
    icon: Option<IconPixels>,
    name: Option<String>,
    missing: bool,
    source: Option<std::path::PathBuf>,
}

#[derive(Clone)]
struct Request {
    path: String,
    generation: u64,
    icon: bool,
    source: Option<std::path::PathBuf>,
    name: Option<String>,
}

struct Loaded {
    request: Request,
    metadata: Metadata,
    complete: bool,
}

pub(crate) struct IconCache {
    images: LruCache<String, Entry>,
    pending: BTreeMap<String, u64>,
    requests: mpsc::SyncSender<Request>,
    results: mpsc::Receiver<Loaded>,
    generation: u64,
    max_age: Option<Duration>,
    stopped: bool,
}

impl IconCache {
    pub fn new() -> Self {
        let mut cache = Self::with_worker(|request, publish| {
            let resolved = crate::platform::foreground_apps::metadata(
                &request.path,
                request.source.as_deref(),
                request.name.as_deref(),
            );
            let details = || Metadata {
                name: resolved.name.clone(),
                missing: resolved.missing,
                source: resolved.path.clone(),
                ..Metadata::default()
            };
            // Name-only requests complete without touching the icon service.
            if publish(details(), !request.icon) && request.icon {
                publish(
                    Metadata {
                        icon: resolved.icon(),
                        ..details()
                    },
                    true,
                );
            }
        });
        cache.max_age = crate::platform::foreground_apps::CACHE_MAX_AGE;
        cache
    }

    #[cfg(test)]
    fn with_loader(loader: impl Fn(&str) -> Metadata + Send + 'static) -> Self {
        Self::with_worker(move |request, publish| {
            publish(loader(&request.path), true);
        })
    }

    fn with_worker(
        loader: impl Fn(&Request, &mut dyn FnMut(Metadata, bool) -> bool) + Send + 'static,
    ) -> Self {
        let (requests, rx) = mpsc::sync_channel::<Request>(64);
        let (tx, results) = mpsc::sync_channel(64);
        if let Err(error) = std::thread::Builder::new()
            .name("taprelay-icons".into())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    let mut connected = true;
                    loader(&request, &mut |metadata, complete| {
                        connected = tx
                            .send(Loaded {
                                request: request.clone(),
                                metadata,
                                complete,
                            })
                            .is_ok();
                        connected
                    });
                    if !connected {
                        break;
                    }
                }
            })
        {
            tracing::error!(%error, "Failed to start application metadata worker");
        }
        Self {
            images: LruCache::new(NonZeroUsize::new(512).unwrap()),
            pending: BTreeMap::new(),
            requests,
            results,
            generation: 0,
            max_age: None,
            stopped: false,
        }
    }

    pub fn seed(&mut self, app: crate::platform::foreground_apps::RunningApp) {
        if app.name.is_none() && app.path.is_none() {
            return;
        }
        let key = executable_identity(&app.identity);
        if !self.images.contains(&key) {
            self.images.put(key.clone(), Entry::default());
        }
        let entry = self.images.get_mut(&key).unwrap();
        if (app.path.is_some() && app.path != entry.source)
            || (app.name.is_some() && app.name != entry.name)
        {
            entry.fresh = false;
            entry.loaded_at = None;
            // Reject any in-flight result for the previous path/name.
            self.pending.remove(&key);
        }
        if app.name.is_some() {
            entry.name = app.name;
        }
        if app.path.is_some() {
            entry.source = app.path;
        }
        entry.missing = false;
    }

    fn expired(entry: &Entry, max_age: Option<Duration>) -> bool {
        max_age.is_some_and(|max_age| {
            let max_age = if entry.missing || (entry.icon_loaded && entry.image.is_none()) {
                max_age.min(Duration::from_secs(5))
            } else {
                max_age
            };
            entry
                .loaded_at
                .is_none_or(|loaded| loaded.elapsed() >= max_age)
        })
    }

    pub fn request_name(&mut self, path: &str) {
        self.request_details(path, false);
    }

    pub fn request(&mut self, path: &str) {
        self.request_details(path, true);
    }

    fn request_details(&mut self, path: &str, icon: bool) {
        if path.is_empty() || self.stopped {
            return;
        }
        let key = executable_identity(path);
        let entry = self.images.get_mut(&key);
        if entry.as_ref().is_some_and(|entry| {
            entry.fresh && (!icon || entry.icon_loaded) && !Self::expired(entry, self.max_age)
        }) || self.pending.contains_key(&key)
        {
            return;
        }
        let request = Request {
            path: path.to_owned(),
            generation: self.generation,
            icon,
            source: entry.as_ref().and_then(|entry| entry.source.clone()),
            name: entry
                .as_ref()
                .filter(|entry| entry.loaded_at.is_none())
                .and_then(|entry| entry.name.clone()),
        };
        if self.requests.try_send(request).is_ok() {
            self.pending.insert(key, self.generation);
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn image(&self, path: &str) -> Option<Arc<RenderImage>> {
        self.images
            .peek(&executable_identity(path))
            .and_then(|entry| entry.image.clone())
    }

    /// Windows rechecks on editor open; macOS reuses metadata for up to a minute.
    /// Keep old presentation while expired entries reload in the background.
    pub fn refresh(&mut self) {
        for (_, entry) in self.images.iter_mut() {
            if self.max_age.is_none() || Self::expired(entry, self.max_age) {
                entry.fresh = false;
            }
        }
    }

    pub fn name(&self, path: &str) -> Option<&str> {
        self.images
            .peek(&executable_identity(path))
            .and_then(|entry| entry.name.as_deref())
    }

    pub fn missing(&self, path: &str) -> bool {
        self.images
            .peek(&executable_identity(path))
            .is_some_and(|entry| entry.missing)
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let Loaded {
                request,
                metadata,
                complete,
            } = match self.results.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.stopped {
                        tracing::error!(
                            "Application metadata worker stopped; cached metadata remains available"
                        );
                        self.stopped = true;
                        self.pending.clear();
                    }
                    break;
                }
            };
            let key = executable_identity(&request.path);
            if self.pending.get(&key) != Some(&request.generation) {
                continue;
            }
            if !self.images.contains(&key) {
                self.images.put(key.clone(), Entry::default());
            }
            let entry = self.images.get_mut(&key).unwrap();
            entry.name = metadata.name;
            entry.missing = metadata.missing;
            entry.source = metadata.source;
            if complete {
                self.pending.remove(&key);
                entry.icon_loaded = request.icon;
                if request.icon {
                    entry.image = metadata.icon.and_then(|mut pixels| {
                        // GPUI consumes BGRA; both native adapters return straight-alpha RGBA.
                        for pixel in pixels.rgba.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        let buffer =
                            image::RgbaImage::from_raw(pixels.size, pixels.size, pixels.rgba)?;
                        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
                    });
                }
                entry.loaded_at = Some(Instant::now());
                entry.fresh = true;
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_result(cache: &mut IconCache) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !cache.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn summary_requests_skip_icons_and_editor_requests_upgrade_cached_names() {
        let (requests, observed) = mpsc::channel();
        let mut cache = IconCache::with_worker(move |request, publish| {
            requests.send(request.icon).unwrap();
            publish(
                Metadata {
                    name: Some("Friendly application".into()),
                    icon: request.icon.then(test_icon),
                    ..Default::default()
                },
                true,
            );
        });
        cache.max_age = Some(Duration::from_secs(60));
        cache.request_name("app");
        assert!(!observed.recv_timeout(Duration::from_secs(3)).unwrap());
        wait_for_result(&mut cache);
        assert_eq!(cache.name("app"), Some("Friendly application"));
        assert!(cache.image("app").is_none());
        // Name-only entries are not failed icon loads; they keep the normal TTL.
        cache.images.get_mut("app").unwrap().loaded_at =
            Some(Instant::now() - Duration::from_secs(6));
        cache.request_name("app");
        assert!(cache.pending.is_empty());
        cache.request("app");
        assert!(observed.recv_timeout(Duration::from_secs(3)).unwrap());
        wait_for_result(&mut cache);
        assert!(cache.image("app").is_some());
        cache.request_name("app");
        cache.request("app");
        assert!(cache.pending.is_empty());
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn a_stopped_worker_clears_pending_requests_without_discarding_cached_names() {
        let mut cache = IconCache::with_loader(|_| Metadata {
            name: Some("Cached".into()),
            ..Default::default()
        });
        cache.request_name("app");
        wait_for_result(&mut cache);
        let (sender, receiver) = mpsc::channel();
        cache.results = receiver;
        drop(sender);
        cache.pending.insert("other".into(), 42);
        assert!(!cache.poll());
        assert!(cache.stopped && cache.pending.is_empty());
        assert_eq!(cache.name("app"), Some("Cached"));
        cache.request("other");
        assert!(cache.pending.is_empty());
    }

    #[test]
    fn capacity_evicts_the_least_requested_entry_not_the_last_rendered_one() {
        let mut cache = IconCache::with_loader(|path| Metadata {
            name: Some(path.into()),
            missing: true,
            ..Metadata::default()
        });
        for index in 0..512 {
            cache.request(&format!(r"C:\Apps\{index}.exe"));
            wait_for_result(&mut cache);
        }
        cache.request("c:/apps/0.exe");
        // Rendering must not promote an entry; only requesting it does.
        assert!(cache.name(r"C:\Apps\1.exe").is_some());
        assert!(cache.missing(r"C:\Apps\1.exe"));
        assert!(cache.image(r"C:\Apps\1.exe").is_none());
        cache.request(r"C:\Apps\512.exe");
        wait_for_result(&mut cache);
        assert_eq!(cache.images.len(), 512);
        assert!(cache.name(r"C:\Apps\0.exe").is_some());
        assert!(cache.name(r"C:\Apps\1.exe").is_none());
        assert!(cache.name(r"C:\Apps\512.exe").is_some());
    }

    #[test]
    fn refreshing_keeps_old_metadata_until_the_background_result_arrives() {
        let (release, waiting) = mpsc::channel();
        let mut cache = IconCache::with_loader(move |_| Metadata {
            name: Some(waiting.recv().unwrap()),
            ..Metadata::default()
        });
        cache.request(r"C:\Apps\one.exe");
        release.send("Old name".to_string()).unwrap();
        wait_for_result(&mut cache);
        cache.refresh();
        cache.request(r"C:\Apps\one.exe");
        assert!(!cache.poll());
        assert_eq!(cache.name(r"C:\Apps\one.exe"), Some("Old name"));
        release.send("New name".to_string()).unwrap();
        wait_for_result(&mut cache);
        assert_eq!(cache.name(r"C:\Apps\one.exe"), Some("New name"));
    }
    #[test]
    fn completed_pixels_are_published_as_a_cached_image() {
        let mut cache = IconCache::with_loader(|_| Metadata {
            icon: Some(IconPixels {
                size: 1,
                rgba: vec![80, 120, 200, 255],
            }),
            name: Some("Friendly app name".into()),
            ..Metadata::default()
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
    fn test_icon() -> IconPixels {
        IconPixels {
            size: 1,
            rgba: vec![80, 120, 200, 255],
        }
    }

    #[test]
    fn metadata_is_published_while_the_icon_is_still_loading() {
        let (release, wait) = mpsc::channel();
        let mut cache = IconCache::with_worker(move |_, publish| {
            assert!(publish(
                Metadata {
                    name: Some("Ready name".into()),
                    ..Metadata::default()
                },
                false
            ));
            wait.recv().unwrap();
            publish(
                Metadata {
                    name: Some("Ready name".into()),
                    icon: Some(test_icon()),
                    ..Metadata::default()
                },
                true,
            );
        });
        cache.request("app");
        wait_for_result(&mut cache);
        assert_eq!(cache.name("app"), Some("Ready name"));
        assert!(cache.image("app").is_none());
        assert_eq!(cache.pending.len(), 1);
        cache.request("app");
        assert_eq!(cache.pending.len(), 1);
        release.send(()).unwrap();
        wait_for_result(&mut cache);
        assert!(cache.image("app").is_some());
        assert!(cache.pending.is_empty());
    }

    #[test]
    fn reopening_reuses_fresh_metadata_and_expiry_reloads_without_clearing_it() {
        let mut cache = IconCache::with_loader(|_| Metadata {
            icon: Some(test_icon()),
            name: Some("Name".into()),
            ..Metadata::default()
        });
        cache.max_age = Some(Duration::from_secs(60));
        cache.request("app");
        wait_for_result(&mut cache);
        let old = cache.image("app").unwrap();
        cache.refresh();
        cache.request("app");
        assert!(cache.pending.is_empty());
        cache.images.get_mut("app").unwrap().loaded_at =
            Some(Instant::now() - Duration::from_secs(61));
        cache.refresh();
        cache.request("app");
        assert_eq!(cache.pending.len(), 1);
        assert!(Arc::ptr_eq(&old, &cache.image("app").unwrap()));
        assert_eq!(cache.name("app"), Some("Name"));
        wait_for_result(&mut cache);
        assert!(cache.pending.is_empty());
    }

    #[test]
    fn failed_metadata_retries_before_successful_cache_entries_expire() {
        let mut cache = IconCache::with_loader(|_| Metadata {
            missing: true,
            ..Metadata::default()
        });
        cache.max_age = Some(Duration::from_secs(60));
        cache.request("app");
        wait_for_result(&mut cache);
        cache.request("app");
        assert!(cache.pending.is_empty());
        cache.images.get_mut("app").unwrap().loaded_at =
            Some(Instant::now() - Duration::from_secs(6));
        cache.request("app");
        assert_eq!(cache.pending.len(), 1);
        wait_for_result(&mut cache);
    }

    #[test]
    fn discovery_names_are_immediate_and_old_path_results_cannot_replace_them() {
        let (release, wait) = mpsc::channel();
        let (started, loading) = mpsc::channel();
        let mut cache = IconCache::with_worker(move |request, publish| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            publish(
                Metadata {
                    name: request.name.clone(),
                    source: request.source.clone(),
                    icon: Some(test_icon()),
                    ..Metadata::default()
                },
                true,
            );
        });
        let app = |name: &str, path: &str| crate::platform::foreground_apps::RunningApp {
            identity: "bundle:org.taprelay.example".into(),
            name: Some(name.into()),
            path: Some(path.into()),
        };
        let id = "bundle:org.taprelay.example";
        cache.seed(app("Old", "/Old.app"));
        assert_eq!(cache.name(id), Some("Old"));
        cache.request(id);
        loading.recv_timeout(Duration::from_secs(3)).unwrap();
        cache.seed(app("New", "/New.app"));
        assert_eq!(cache.name(id), Some("New"));
        cache.request(id);
        release.send(()).unwrap();
        loading.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!cache.poll(), "stale result must be discarded");
        assert_eq!(cache.name(id), Some("New"));
        assert!(cache.image(id).is_none());
        release.send(()).unwrap();
        wait_for_result(&mut cache);
        assert_eq!(cache.name(id), Some("New"));
        assert_eq!(
            cache.images.peek(id).unwrap().source.as_deref(),
            Some(std::path::Path::new("/New.app"))
        );
        assert!(cache.image(id).is_some());
    }
}
