use serde::Serialize;
use std::io::Write;
use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;

use super::rate_limiter::{CheckResult, RateLimiter};

struct FileLoggerState {
    writer: std::sync::Mutex<tracing_appender::non_blocking::NonBlocking>,
    _guard: WorkerGuard,
    limiter: RateLimiter,
    version: String,
    commit: String,
}

static STATE: OnceLock<FileLoggerState> = OnceLock::new();

pub fn init() {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit = env!("GIT_COMMIT_HASH").to_string();
    let log_dir = super::platform::log_dir();
    let suppressed_path = crate::paths::suppressed_errors_path()
        .unwrap_or_else(|_| log_dir.join("suppressed-errors.json"));

    let version_tag = format!("v{}@{}", version, commit);
    let limiter = RateLimiter::load(&suppressed_path, version_tag);

    let build_appender = |dir: &std::path::Path| {
        tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("qol-tray")
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir)
    };

    let file_appender = match build_appender(&log_dir) {
        Ok(appender) => appender,
        Err(primary) => {
            eprintln!(
                "Failed to create log file appender at {}: {primary}",
                log_dir.display()
            );
            let fallback = std::env::temp_dir().join("qol-tray/logs");
            match build_appender(&fallback) {
                Ok(appender) => appender,
                Err(secondary) => {
                    eprintln!(
                        "Fallback log dir {} also failed: {secondary}",
                        fallback.display()
                    );
                    return;
                }
            }
        }
    };

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let _ = STATE.set(FileLoggerState {
        writer: std::sync::Mutex::new(non_blocking),
        _guard: guard,
        limiter,
        version,
        commit,
    });
}

pub fn on_error_event(target: &str, message: &str, file: &str, line: u32) {
    let Some(state) = STATE.get() else {
        return;
    };

    let key = format!("{}:{}:{}", target, file, line);
    let result = state.limiter.check(&key);
    let (count, suppressed) = match result {
        CheckResult::Rejected => return,
        CheckResult::Allowed { count } => (count, false),
        CheckResult::Suppressed { count } => (count, true),
    };

    let src = target.strip_prefix("qol_tray::").unwrap_or(target);

    let entry = LogEntry {
        ts: now_iso(),
        level: "error",
        v: &state.version,
        commit: &state.commit,
        src,
        key: &key,
        msg: message,
        count,
        suppressed,
        loc: &format!("{}:{}", file, line),
    };
    write_jsonl(state, &entry);

    if suppressed {
        save_suppressed(&state.limiter);
    }

    state
        .limiter
        .update_entry_context(&key, message, src, &format!("{}:{}", file, line));
}

pub fn log_entry(key: &str, source: &str, message: &str, file: &str, line: u32) {
    let Some(state) = STATE.get() else {
        return;
    };

    let result = state.limiter.check(key);
    let (count, suppressed) = match result {
        CheckResult::Rejected => return,
        CheckResult::Allowed { count } => (count, false),
        CheckResult::Suppressed { count } => (count, true),
    };

    let entry = LogEntry {
        ts: now_iso(),
        level: "error",
        v: &state.version,
        commit: &state.commit,
        src: source,
        key,
        msg: message,
        count,
        suppressed,
        loc: &format!("{}:{}", file, line),
    };
    write_jsonl(state, &entry);

    if suppressed {
        save_suppressed(&state.limiter);
    }

    state
        .limiter
        .update_entry_context(key, message, source, &format!("{}:{}", file, line));
}

pub fn log_startup(info: &str) {
    let Some(state) = STATE.get() else {
        return;
    };
    let entry = LogEntry {
        ts: now_iso(),
        level: "startup",
        v: &state.version,
        commit: &state.commit,
        src: "core",
        key: "startup",
        msg: info,
        count: 1,
        suppressed: false,
        loc: "",
    };
    write_jsonl(state, &entry);
}

#[derive(Serialize)]
struct LogEntry<'a> {
    ts: String,
    level: &'a str,
    v: &'a str,
    commit: &'a str,
    src: &'a str,
    key: &'a str,
    msg: &'a str,
    count: u32,
    suppressed: bool,
    loc: &'a str,
}

fn write_jsonl(state: &FileLoggerState, entry: &LogEntry<'_>) {
    let Ok(mut json) = serde_json::to_string(entry) else {
        return;
    };
    json.push('\n');
    let Ok(mut writer) = state.writer.lock() else {
        return;
    };
    let _ = writer.write_all(json.as_bytes());
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub fn unsuppress_key(key: &str) {
    let Some(state) = STATE.get() else {
        return;
    };
    state.limiter.unsuppress(key);
    save_suppressed(&state.limiter);
}

fn save_suppressed(limiter: &RateLimiter) {
    if let Ok(path) = crate::paths::suppressed_errors_path() {
        limiter.save(&path);
    }
}

#[macro_export]
macro_rules! log_error {
    ($key:expr, source = $source:expr, $($arg:tt)+) => {
        $crate::logging::file_logger::log_entry(
            $key,
            &$source,
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
    ($key:expr, $($arg:tt)+) => {
        $crate::logging::file_logger::log_entry(
            $key,
            "core",
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
}
