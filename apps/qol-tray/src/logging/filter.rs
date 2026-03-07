use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::LogControl;

pub type CoreControlsHandle = Arc<RwLock<HashMap<String, LogControl>>>;

struct Section {
    name: &'static str,
    prefix: &'static str,
}

const SECTIONS: &[Section] = &[
    Section {
        name: "runtime",
        prefix: "qol_tray::runtime",
    },
    Section {
        name: "plugins",
        prefix: "qol_tray::plugins",
    },
];

const CATCH_ALL: &str = "core";

fn section_for_target(target: &str) -> Option<&'static str> {
    for section in SECTIONS {
        if target.starts_with(section.prefix) {
            return Some(section.name);
        }
    }
    if target.starts_with("qol_tray") {
        return Some(CATCH_ALL);
    }
    None
}

struct FilterableLogger {
    inner: env_logger::Logger,
    controls: CoreControlsHandle,
}

impl log::Log for FilterableLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if !self.inner.enabled(record.metadata()) {
            return;
        }
        if self.is_suppressed(record) {
            return;
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

impl FilterableLogger {
    fn is_suppressed(&self, record: &log::Record) -> bool {
        let Some(section) = section_for_target(record.target()) else {
            return false;
        };
        let controls = self.controls.read().unwrap_or_else(|e| e.into_inner());
        let Some(control) = controls.get(section) else {
            return false;
        };
        if control.muted {
            return true;
        }
        if control.suppress_patterns.is_empty() {
            return false;
        }
        let message = record.args().to_string();
        super::control::matches_any_pattern(&message, &control.suppress_patterns)
    }
}

pub(super) fn init(controls: CoreControlsHandle) {
    let inner =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let max_level = inner.filter();
    let logger = FilterableLogger { inner, controls };
    log::set_boxed_logger(Box::new(logger)).expect("Logger already initialized");
    log::set_max_level(max_level);
}
