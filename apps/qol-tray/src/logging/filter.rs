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

pub(super) fn is_suppressed(controls: &CoreControlsHandle, target: &str, message: &str) -> bool {
    let Some(section) = section_for_target(target) else {
        return false;
    };
    let controls = controls.read().unwrap_or_else(|e| e.into_inner());
    let Some(control) = controls.get(section) else {
        return false;
    };
    if control.muted {
        return true;
    }
    if control.suppress_patterns.is_empty() {
        return false;
    }
    super::control::matches_any_pattern(message, &control.suppress_patterns)
}
