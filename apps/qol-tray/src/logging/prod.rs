use serde::Serialize;
use std::io::Write;
use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;

use super::rate_limiter::{CheckResult, RateLimiter};

struct ProdState {
    writer: std::sync::Mutex<tracing_appender::non_blocking::NonBlocking>,
    _guard: WorkerGuard,
    limiter: RateLimiter,
    version: String,
    commit: String,
}

static STATE: OnceLock<ProdState> = OnceLock::new();

pub fn init() {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit = env!("GIT_COMMIT_HASH").to_string();
    let log_dir = super::platform::log_dir();
    let suppressed_path = crate::paths::suppressed_errors_path()
        .unwrap_or_else(|_| log_dir.join("suppressed-errors.json"));

    let version_tag = format!("v{}@{}", version, commit);
    let limiter = RateLimiter::load(&suppressed_path, version_tag);

    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("qol-tray")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&log_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create log file appender: {}", e);
            tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("qol-tray")
                .filename_suffix("log")
                .max_log_files(7)
                .build("/tmp/qol-tray/logs")
                .expect("fallback log dir also failed")
        });

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let _ = STATE.set(ProdState {
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

fn write_jsonl(state: &ProdState, entry: &LogEntry<'_>) {
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

fn save_suppressed(limiter: &RateLimiter) {
    if let Ok(path) = crate::paths::suppressed_errors_path() {
        limiter.save(&path);
    }
}

#[macro_export]
macro_rules! log_error {
    ($key:expr, source = $source:expr, $($arg:tt)+) => {
        $crate::logging::prod::log_entry(
            $key,
            &$source,
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
    ($key:expr, $($arg:tt)+) => {
        $crate::logging::prod::log_entry(
            $key,
            "core",
            &format!($($arg)+),
            file!(),
            line!(),
        )
    };
}
