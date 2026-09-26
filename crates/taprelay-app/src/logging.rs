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
                "version": crate::version::VERSION,
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
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn selected_level_filters_app_and_worker_logs_without_changing_severity() {
        const CHILD: &str = "TAPRELAY_LEVEL_TEST_DIR";
        const LEVEL: &str = "TAPRELAY_LEVEL_TEST_VALUE";
        if let Some(dir) = std::env::var_os(CHILD) {
            let requested = std::env::var(LEVEL).unwrap();
            configure_level(parse_level(&requested).unwrap()).unwrap();
            assert_eq!(arguments(), ["--log-level".to_owned(), requested]);
            let (_, guard) = init(&Path::new(&dir).join("config.json")).unwrap();
            tracing::error!("app error");
            tracing::warn!("app warn");
            tracing::info!("app info");
            tracing::debug!("app debug");
            tracing::trace!("app trace");
            for level in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
                forward_worker_line(
                    &serde_json::json!({"level": level, "fields": {"message": "worker"}})
                        .to_string(),
                );
            }
            forward_worker_line("unstructured worker failure");
            drop(guard);
            return;
        }
        for (count, requested) in ["off", "error", "warn", "info", "debug", "trace"]
            .into_iter()
            .enumerate()
        {
            let dir = tempfile::tempdir().unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "logging::tests::selected_level_filters_app_and_worker_logs_without_changing_severity", "--nocapture"])
                .env(CHILD, dir.path()).env(LEVEL, requested).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let file = std::fs::read_dir(dir.path().join("logs"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let text = std::fs::read_to_string(file).unwrap();
            let records: Vec<serde_json::Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                records.len(),
                count * 2 + usize::from(count > 0),
                "{requested}"
            );
            for record in records {
                let actual = record["level"].as_str().unwrap();
                assert!(parse_level(actual).unwrap() <= parse_level(requested).unwrap());
                if let Some(worker) = record["fields"]["worker"].as_str() {
                    if let Ok(worker) = serde_json::from_str::<serde_json::Value>(worker) {
                        assert_eq!(record["level"], worker["level"]);
                    } else {
                        assert_eq!(actual, "ERROR");
                    }
                }
            }
        }
    }

    #[test]
    fn panic_is_written_without_a_tracing_subscriber() {
        const CHILD: &str = "TAPRELAY_PANIC_TEST_DIR";
        if let Some(dir) = std::env::var_os(CHILD) {
            panic_handler_in(dir.into());
            panic!("intentional emergency logging test\n中文 \"quoted\" \\");
        }
        let dir = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::panic_is_written_without_a_tracing_subscriber",
                "--nocapture",
            ])
            .env(CHILD, dir.path())
            .env_remove("RUST_BACKTRACE")
            .env_remove("RUST_LIB_BACKTRACE")
            .output()
            .unwrap();
        assert!(!result.status.success());
        let log = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(log.extension().unwrap(), "jsonl");
        assert_eq!(text.lines().count(), 1);
        let record: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(
            record["fields"]["message"],
            "intentional emergency logging test\n中文 \"quoted\" \\"
        );
        for field in ["location", "thread", "version"] {
            assert!(record["fields"][field].is_string());
        }
        let backtrace = record["fields"]["backtrace"].as_str().unwrap();
        assert!(!backtrace.is_empty());
        assert!(!backtrace.contains("disabled backtrace"));
        assert!(backtrace.contains("panic_is_written_without_a_tracing_subscriber"));
    }
    #[test]
    fn retention_preserves_recent_and_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        for prefix in ["taprelay", "taprelay-panic"] {
            for ext in ["log", "jsonl"] {
                for date in ["2026-08-30", "2026-08-31", "2026-09-07"] {
                    std::fs::write(dir.path().join(format!("{prefix}.{date}.{ext}")), "x").unwrap();
                }
            }
        }
        for name in ["personal.log", "personal.jsonl", "taprelay.invalid.jsonl"] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        cleanup(dir.path(), time::macros::date!(2026 - 09 - 07)).unwrap();
        for prefix in ["taprelay", "taprelay-panic"] {
            for ext in ["log", "jsonl"] {
                for (date, retained) in [
                    ("2026-08-30", false),
                    ("2026-08-31", true),
                    ("2026-09-07", true),
                ] {
                    assert_eq!(
                        dir.path().join(format!("{prefix}.{date}.{ext}")).exists(),
                        retained
                    );
                }
            }
        }
        for name in ["personal.log", "personal.jsonl", "taprelay.invalid.jsonl"] {
            assert!(dir.path().join(name).exists());
        }
    }

    #[test]
    fn jsonl_preserves_fields_and_flushes_on_shutdown() {
        const CHILD: &str = "TAPRELAY_JSONL_TEST_DIR";
        const MESSAGE: &str = "中文 \"quoted\" \\ path\nsecond line";
        if let Some(dir) = std::env::var_os(CHILD) {
            let (counter, guard) = init(&Path::new(&dir).join("config.json")).unwrap();
            {
                let span = tracing::info_span!("operation", device = "tablet");
                let _entered = span.enter();
                tracing::info!(count = 42_u64, enabled = true, "{MESSAGE}");
                tracing::warn!("warning");
                tracing::error!("failure");
                tracing::debug!("application debug detail");
                tracing::trace!(target: "wgpu_core::device::queue", "render submission");
                tracing::debug!(target: "naga", "shader detail");
                tracing::info!(target: "wgpu_hal", "adapter detail");
                tracing::warn!(target: "wgpu_hal", "driver warning");
            }
            assert_eq!(counter.dropped_lines(), 0);
            drop(guard);
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::jsonl_preserves_fields_and_flushes_on_shutdown",
                "--nocapture",
            ])
            .env(CHILD, dir.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let paths: Vec<_> = std::fs::read_dir(dir.path().join("logs"))
            .unwrap()
            .map(|item| item.unwrap().path())
            .collect();
        assert!(
            paths
                .iter()
                .all(|path| path.extension().unwrap() == "jsonl")
        );
        let records: Vec<serde_json::Value> = paths
            .iter()
            .flat_map(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                assert!(text.ends_with('\n'));
                text.lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(records.len(), 5);
        let info = records
            .iter()
            .find(|record| record["level"] == "INFO")
            .unwrap();
        assert!(info["timestamp"].is_string());
        assert!(info["target"].is_string());
        assert_eq!(info["fields"]["message"], MESSAGE);
        assert_eq!(info["fields"]["count"], 42);
        assert_eq!(info["fields"]["enabled"], true);
        assert_eq!(info["span"]["device"], "tablet");
        assert_eq!(info["spans"][0]["name"], "operation");
        assert!(records.iter().any(|record| record["level"] == "WARN"));
        assert!(records.iter().any(|record| record["level"] == "ERROR"));
        assert!(
            records
                .iter()
                .any(|record| record["fields"]["message"] == "driver warning")
        );
    }
}
