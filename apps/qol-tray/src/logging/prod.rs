use serde::Serialize;
use std::sync::OnceLock;

use super::rate_limiter::{CheckResult, RateLimiter};
use super::writer::{self, LogWriter};

struct ProdLogger {
    inner: Box<dyn log::Log>,
    writer: LogWriter,
    limiter: RateLimiter,
    version: String,
    commit: String,
}

unsafe impl Sync for ProdLogger {}

static LOGGER: OnceLock<ProdLogger> = OnceLock::new();

pub fn init_with_inner(inner: Box<dyn log::Log>, max_level: log::LevelFilter) {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let commit = env!("GIT_COMMIT_HASH").to_string();
    let log_dir = super::platform::log_dir();
    let suppressed_path = crate::paths::suppressed_errors_path()
        .unwrap_or_else(|_| log_dir.join("suppressed-errors.json"));

    writer::rotate_old_logs(&log_dir, 7);

    let version_tag = format!("v{}@{}", version, commit);
    let limiter = RateLimiter::load(&suppressed_path, version_tag);
    let file_writer = LogWriter::new(log_dir);

    let logger = ProdLogger {
        inner,
        writer: file_writer,
        limiter,
        version,
        commit,
    };

    let _ = LOGGER.set(logger);
    let prod = LOGGER.get().unwrap();
    log::set_logger(prod).expect("logger already initialized");
    log::set_max_level(max_level);
}

impl log::Log for ProdLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if self.inner.enabled(record.metadata()) {
            self.inner.log(record);
        }

        if record.level() <= log::Level::Error {
            self.capture_to_file(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

impl ProdLogger {
    fn capture_to_file(&self, record: &log::Record) {
        let file = record.file().unwrap_or("unknown");
        let line = record.line().unwrap_or(0);
        let key = format!("{}:{}:{}", record.target(), file, line);
        let message = record.args().to_string();

        let result = self.limiter.check(&key);
        let (count, suppressed) = match result {
            CheckResult::Rejected => return,
            CheckResult::Allowed { count } => (count, false),
            CheckResult::Suppressed { count } => (count, true),
        };

        let src = simplify_target(record.target());
        let entry = LogEntry {
            ts: now_iso(),
            level: "error",
            v: &self.version,
            commit: &self.commit,
            src: &src,
            key: &key,
            msg: &message,
            count,
            suppressed,
            loc: &format!("{}:{}", file, line),
        };
        write_jsonl(&self.writer, &entry);

        if suppressed {
            save_suppressed(&self.limiter);
        }

        self.limiter
            .update_entry_context(&key, &message, &src, &format!("{}:{}", file, line));
    }
}

fn simplify_target(target: &str) -> String {
    target
        .strip_prefix("qol_tray::")
        .unwrap_or(target)
        .to_string()
}

pub fn log_entry(key: &str, source: &str, message: &str, file: &str, line: u32) {
    let Some(logger) = LOGGER.get() else {
        return;
    };

    let result = logger.limiter.check(key);
    let (count, suppressed) = match result {
        CheckResult::Rejected => return,
        CheckResult::Allowed { count } => (count, false),
        CheckResult::Suppressed { count } => (count, true),
    };

    let entry = LogEntry {
        ts: now_iso(),
        level: "error",
        v: &logger.version,
        commit: &logger.commit,
        src: source,
        key,
        msg: message,
        count,
        suppressed,
        loc: &format!("{}:{}", file, line),
    };
    write_jsonl(&logger.writer, &entry);

    if suppressed {
        save_suppressed(&logger.limiter);
    }

    logger
        .limiter
        .update_entry_context(key, message, source, &format!("{}:{}", file, line));
}

pub fn log_startup(info: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let entry = LogEntry {
        ts: now_iso(),
        level: "startup",
        v: &logger.version,
        commit: &logger.commit,
        src: "core",
        key: "startup",
        msg: info,
        count: 1,
        suppressed: false,
        loc: "",
    };
    write_jsonl(&logger.writer, &entry);
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

fn write_jsonl(writer: &LogWriter, entry: &LogEntry<'_>) {
    let Ok(mut json) = serde_json::to_string(entry) else {
        return;
    };
    json.push('\n');
    writer.write(&json);
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
