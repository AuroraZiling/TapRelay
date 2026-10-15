use std::path::Path;
pub use tracing_subscriber::filter::LevelFilter as LogLevel;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static LEVEL: std::sync::OnceLock<LogLevel> = std::sync::OnceLock::new();

pub fn parse_level(value: &str) -> anyhow::Result<LogLevel> {
    match value.to_ascii_lowercase().as_str() {
        "off" => Ok(LogLevel::OFF),
        "error" => Ok(LogLevel::ERROR),
        "warn" => Ok(LogLevel::WARN),
        "info" => Ok(LogLevel::INFO),
        "debug" => Ok(LogLevel::DEBUG),
        "trace" => Ok(LogLevel::TRACE),
        _ => anyhow::bail!(
            "Invalid --log-level `{value}`. Expected off, error, warn, info, debug or trace."
        ),
    }
}

pub fn configure_level(level: LogLevel) -> anyhow::Result<()> {
    LEVEL
        .set(level)
        .map_err(|_| anyhow::anyhow!("Logging level already configured"))
}

fn level() -> LogLevel {
    LEVEL.get().copied().unwrap_or(LogLevel::INFO)
}

#[cfg(windows)]
pub fn arguments() -> [String; 2] {
    [
        "--log-level".into(),
        level().to_string().to_ascii_lowercase(),
    ]
}

pub fn init(
    path: &Path,
) -> anyhow::Result<(
    tracing_appender::non_blocking::ErrorCounter,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    let dir = path.parent().unwrap_or(Path::new(".")).join("logs");
    std::fs::create_dir_all(&dir)?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("taprelay")
        .filename_suffix("jsonl")
        .build(&dir)?;
    cleanup(&dir, time::OffsetDateTime::now_utc().date())?;
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .finish(file);
    let counter = writer.error_counter();
    tracing_subscriber::registry()
        .with(level())
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init()?;
    Ok((counter, guard))
}

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
            .and_then(|s| s.strip_suffix(".jsonl").or_else(|| s.strip_suffix(".log")))
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

#[cfg(windows)]
pub fn init_worker(args: &[String]) -> anyhow::Result<()> {
    let requested = match args {
        [] => LogLevel::INFO,
        [flag, value] if flag == "--log-level" => parse_level(value)?,
        _ => anyhow::bail!("Invalid Bluetooth worker arguments; expected --log-level <level>"),
    };
    configure_level(requested)?;
    tracing_subscriber::registry()
        .with(level())
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr),
        )
        .try_init()?;
    Ok(())
}

#[cfg(windows)]
pub fn forward_worker_line(line: &str) {
    let level = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|record| parse_level(record.get("level")?.as_str()?).ok())
        // Unstructured stderr may be a panic or startup failure.
        .unwrap_or(LogLevel::ERROR);
    match level {
        LogLevel::ERROR => tracing::error!(worker = line, "Bluetooth worker"),
        LogLevel::WARN => tracing::warn!(worker = line, "Bluetooth worker"),
        LogLevel::INFO => tracing::info!(worker = line, "Bluetooth worker"),
        LogLevel::DEBUG => tracing::debug!(worker = line, "Bluetooth worker"),
        LogLevel::TRACE => tracing::trace!(worker = line, "Bluetooth worker"),
        _ => {}
    }
}

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
        let message = serde_json::json!({
            "timestamp": now.format(&time::format_description::well_known::Rfc3339).unwrap_or_else(|_| now.to_string()),
            "level": "ERROR",
            "target": "taprelay::panic",
            "fields": {
                "message": payload,
                "version": crate::constants::VERSION,
                "thread": thread.name().unwrap_or("unnamed"),
                "location": info.location().map(ToString::to_string).unwrap_or_else(|| "unknown".into()),
                "backtrace": std::backtrace::Backtrace::force_capture().to_string()
            }
        });
        if std::fs::create_dir_all(&dir).is_ok()
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("taprelay-panic.{}.jsonl", now.date())))
        {
            let mut record = serde_json::to_vec(&message).expect("JSON value serialization");
            record.push(b'\n');
            let _ = file.write_all(&record);
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
mod tests;
