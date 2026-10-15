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
                &serde_json::json!({"level": level, "fields": {"message": "worker"}}).to_string(),
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
