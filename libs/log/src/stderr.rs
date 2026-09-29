use std::io::Write;

struct StderrLogger;

static LOGGER: StderrLogger = StderrLogger;

/// Dependencies such as gpui log routine window setup at info; only their
/// warnings and errors reach a qol log.
const DEPENDENCY_LEVEL: log::Level = log::Level::Warn;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
            && (metadata.target().starts_with("qol") || metadata.level() <= DEPENDENCY_LEVEL)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let _ = writeln!(
            std::io::stderr().lock(),
            "{} {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// Sends this process's `log::*!` records to stderr. Every qol process except
/// qol-tray calls it first thing in `main`; the tray relays plugin stderr into
/// its daemon log. A second call is a no-op.
pub fn init_stderr() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{StderrLogger, DEPENDENCY_LEVEL};
    use log::{Level, Log, Metadata};

    #[test]
    fn dependency_records_below_warn_are_dropped() {
        log::set_max_level(log::LevelFilter::Trace);
        let cases = [
            ("qol_alt_tab::picker", Level::Debug, true),
            ("qol_log", Level::Info, true),
            ("gpui::platform::linux::x11::client", Level::Info, false),
            ("gpui::window", Level::Debug, false),
            ("gpui::window", DEPENDENCY_LEVEL, true),
            ("zbus", Level::Error, true),
        ];
        for (target, level, expected) in cases {
            let metadata = Metadata::builder().target(target).level(level).build();
            assert_eq!(
                StderrLogger.enabled(&metadata),
                expected,
                "{target} at {level}"
            );
        }
    }
}
