use crate::{LogLevel, LogLine};
use std::{
    collections::VecDeque,
    fmt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
const LIMIT: usize = 1000;
/// Severity toggles of the log page. TRACE follows DEBUG because the page
/// exposes four levels only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub debug: bool,
    pub info: bool,
    pub warning: bool,
    pub error: bool,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            debug: false,
            info: true,
            warning: true,
            error: true,
        }
    }
}
impl Filter {
    fn accepts(self, level: tracing::Level) -> bool {
        match level {
            tracing::Level::ERROR => self.error,
            tracing::Level::WARN => self.warning,
            tracing::Level::INFO => self.info,
            _ => self.debug,
        }
    }
}
fn severity(level: tracing::Level) -> LogLevel {
    match level {
        tracing::Level::ERROR => LogLevel::Error,
        tracing::Level::WARN => LogLevel::Warning,
        tracing::Level::INFO => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub level: tracing::Level,
    line: String,
}
#[derive(Clone, Default)]
pub struct Logs {
    lines: Arc<Mutex<VecDeque<Entry>>>,
    pub revision: Arc<AtomicU64>,
    dropped: Option<tracing_appender::non_blocking::ErrorCounter>,
}
impl Logs {
    pub fn dropped_lines(&self) -> usize {
        self.dropped
            .as_ref()
            .map_or(0, |counter| counter.dropped_lines())
    }
    #[cfg(test)]
    pub fn entries(&self) -> Vec<Entry> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
    pub fn filtered(&self, filter: Filter) -> Vec<LogLine> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|entry| filter.accepts(entry.level))
            .map(|entry| LogLine {
                text: entry.line.as_str().into(),
                level: severity(entry.level),
            })
            .collect()
    }
    pub fn clear(&self) {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.revision.fetch_add(1, Ordering::Release);
    }
}
fn timestamp() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
}
#[derive(Default)]
struct Message {
    body: String,
    fields: String,
}
impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        use fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.body, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}
impl<S: Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = Message::default();
        event.record(&mut visitor);
        visitor.body.push_str(&visitor.fields);
        // Strip terminal controls and bound event memory, preserving explicit newlines.
        let message: String = visitor
            .body
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(4096)
            .collect();
        let entry = Entry {
            level: *event.metadata().level(),
            line: format!("{} {:5} {}", timestamp(), event.metadata().level(), message),
        };
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() == LIMIT {
            lines.pop_front();
        }
        lines.push_back(entry);
        self.revision.fetch_add(1, Ordering::Release);
    }
}
pub fn init(path: &Path) -> anyhow::Result<(Logs, tracing_appender::non_blocking::WorkerGuard)> {
    let dir = path.parent().unwrap_or(Path::new(".")).join("logs");
    std::fs::create_dir_all(&dir)?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("taprelay")
        .filename_suffix("log")
        .build(&dir)?;
    cleanup(&dir, time::OffsetDateTime::now_utc().date())?;
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .finish(file);
    let logs = Logs {
        dropped: Some(writer.error_counter()),
        ..Default::default()
    };
    tracing_subscriber::registry()
        .with(logs.clone())
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init()?;
    Ok((logs, guard))
}
/// Retention is date based, not file-count based; sparse logging must also expire.
pub fn cleanup(dir: &Path, today: time::Date) -> anyhow::Result<()> {
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        if !item.file_type()?.is_file() {
            continue;
        }
        let name = item.file_name();
        let name = name.to_string_lossy();
        let Some(date) = name
            .strip_prefix("taprelay.")
            .or_else(|| name.strip_prefix("taprelay-panic."))
            .and_then(|s| s.strip_suffix(".log"))
        else {
            continue;
        };
        let parts: Vec<_> = date.split('-').collect();
        if parts.len() != 3 {
            continue;
        }
        let parsed = (|| {
            time::Date::from_calendar_date(
                parts[0].parse().ok()?,
                time::Month::try_from(parts[1].parse::<u8>().ok()?).ok()?,
                parts[2].parse().ok()?,
            )
            .ok()
        })();
        if parsed.is_some_and(|d| d < today - time::Duration::days(7)) {
            std::fs::remove_file(item.path())?;
        }
    }
    Ok(())
}

/// Independent emergency output: do not re-enter tracing or acquire its locks
/// from a panic hook. Native access violations/abort/OOM require OS crash dumps.
pub fn install_panic_handler() {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        panic_handler_in(parent.join("logs"));
    }
}
fn panic_handler_in(dir: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let thread = std::thread::current();
        let now = time::OffsetDateTime::now_utc();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic");
        let payload: String = payload.chars().take(4096).collect();
        let message = format!(
            "{now} PANIC version={} thread={} location={} reason={payload}\n",
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
            info.location()
                .map(ToString::to_string)
                .unwrap_or_else(|| "unknown".into())
        );
        if std::fs::create_dir_all(&dir).is_ok()
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("taprelay-panic.{}.log", now.date())))
        {
            let _ = file.write_all(message.as_bytes());
            let _ = file.sync_data();
        }
        previous(info);
    }));
}
pub fn cleanup_due(data: &Path) -> anyhow::Result<()> {
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(3600)) {
        *last = Some(std::time::Instant::now());
        cleanup(&data.join("logs"), time::OffsetDateTime::now_utc().date())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panic_is_written_without_a_tracing_subscriber() {
        const CHILD: &str = "TAPRELAY_PANIC_TEST_DIR";
        if let Some(dir) = std::env::var_os(CHILD) {
            panic_handler_in(dir.into());
            panic!("intentional emergency logging test");
        }
        let dir = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::panic_is_written_without_a_tracing_subscriber",
                "--nocapture",
            ])
            .env(CHILD, dir.path())
            .output()
            .unwrap();
        assert!(!result.status.success());
        let log = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(log).unwrap();
        assert!(text.contains("intentional emergency logging test"));
        assert!(
            text.contains("location=") && text.contains("thread=") && text.contains("version=")
        );
    }
    #[test]
    fn retention_preserves_recent_and_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        for f in [
            "taprelay.2026-08-30.log",
            "taprelay.2026-08-31.log",
            "taprelay.2026-09-07.log",
            "personal.log",
        ] {
            std::fs::write(dir.path().join(f), "x").unwrap();
        }
        cleanup(dir.path(), time::macros::date!(2026 - 09 - 07)).unwrap();
        assert!(!dir.path().join("taprelay.2026-08-30.log").exists());
        for f in [
            "taprelay.2026-08-31.log",
            "taprelay.2026-09-07.log",
            "personal.log",
        ] {
            assert!(dir.path().join(f).exists());
        }
    }
    #[test]
    fn severity_is_structured_and_ring_is_bounded() {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..1001 {
                tracing::info!("ERROR is just text");
            }
            tracing::debug!("noise");
            tracing::error!("failure");
        });
        let entries = logs.entries();
        assert_eq!(entries.len(), LIMIT);
        assert!(entries.iter().any(|e| e.level == tracing::Level::DEBUG));
        let lines = logs.filtered(Filter::default());
        assert_eq!(lines.len(), LIMIT - 1);
        assert!(
            lines
                .iter()
                .all(|line| line.level == LogLevel::Info || line.level == LogLevel::Error),
            "Text containing ERROR must not change its severity"
        );
        assert_eq!(
            lines.last().map(|line| line.level),
            Some(LogLevel::Error),
            "Error entries stay visible with the default toggles"
        );
        assert_eq!(
            logs.filtered(Filter {
                debug: true,
                ..Filter::default()
            })
            .len(),
            LIMIT
        );
        logs.clear();
        assert!(logs.entries().is_empty());
        assert!(logs.filtered(Filter::default()).is_empty());
    }
}
