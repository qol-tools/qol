use std::sync::OnceLock;

use super::rate_limiter::{CheckResult, RateLimiter};
use super::writer::{self, LogWriter};

struct ProdLogger {
    writer: LogWriter,
    limiter: RateLimiter,
    version_tag: String,
}

static LOGGER: OnceLock<ProdLogger> = OnceLock::new();

pub fn init() {
    let version = format!("v{}@{}", env!("CARGO_PKG_VERSION"), env!("GIT_COMMIT_HASH"));
    let log_dir = super::platform::log_dir();
    let suppressed_path = crate::paths::suppressed_errors_path()
        .unwrap_or_else(|_| log_dir.join("suppressed-errors.json"));

    writer::rotate_old_logs(&log_dir, 7);

    let limiter = RateLimiter::load(&suppressed_path, version.clone());
    let writer = LogWriter::new(log_dir);

    let _ = LOGGER.set(ProdLogger {
        writer,
        limiter,
        version_tag: version,
    });
}

pub fn log_entry(key: &str, source: &str, message: &str, file: &str, line: u32) {
    let Some(logger) = LOGGER.get() else {
        return;
    };

    let result = logger.limiter.check(key);
    match result {
        CheckResult::Rejected => return,
        CheckResult::Allowed { count } => {
            let count_suffix = if count > 1 {
                format!(" (x{})", count)
            } else {
                String::new()
            };
            let entry = format_entry(
                &logger.version_tag,
                source,
                key,
                message,
                file,
                line,
                &count_suffix,
            );
            logger.writer.write(&entry);
        }
        CheckResult::Suppressed { count } => {
            let entry = format_entry(
                &logger.version_tag,
                source,
                key,
                message,
                file,
                line,
                &format!(" (x{}, suppressed)", count),
            );
            logger.writer.write(&entry);
            save_suppressed(&logger.limiter);
        }
    }

    logger
        .limiter
        .update_entry_context(key, message, source, &format!("{}:{}", file, line));
}

fn format_entry(
    version: &str,
    source: &str,
    key: &str,
    message: &str,
    file: &str,
    line: u32,
    suffix: &str,
) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    format!(
        "[{}] [{}] [{}] ERROR {} — {}{} ({}:{})\n",
        timestamp, version, source, key, message, suffix, file, line
    )
}

fn save_suppressed(limiter: &RateLimiter) {
    if let Ok(path) = crate::paths::suppressed_errors_path() {
        limiter.save(&path);
    }
}

pub fn log_startup(info: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let entry = format!(
        "[{}] [{}] [core] STARTUP — {}\n",
        timestamp, logger.version_tag, info
    );
    logger.writer.write(&entry);
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
