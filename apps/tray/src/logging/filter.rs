use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::LogControl;

pub type CoreControlsHandle = Arc<RwLock<HashMap<String, LogControl>>>;

struct Section {
    name: &'static str,
    prefix: &'static str,
}

pub const CORE_LOG_SECTION_IDS: &[&str] = &[
    "auth",
    "desktop-state",
    "features",
    "hotkeys",
    "launcher-apps",
    "plugin-store",
    "plugins",
    "profile",
    "runtime",
    "shortcuts",
    "task-runner",
    "tray",
    "updates",
    "core",
];

const SECTIONS: &[Section] = &[
    Section {
        name: "auth",
        prefix: "qol_tray::features::auth",
    },
    Section {
        name: "auth",
        prefix: "qol_tray::features::github_auth",
    },
    Section {
        name: "desktop-state",
        prefix: "qol_tray::desktop_state",
    },
    Section {
        name: "hotkeys",
        prefix: "qol_tray::hotkeys",
    },
    Section {
        name: "launcher-apps",
        prefix: "qol_tray::features::launcher_apps",
    },
    Section {
        name: "plugin-store",
        prefix: "qol_tray::features::plugin_store",
    },
    Section {
        name: "plugins",
        prefix: "qol_tray::plugins",
    },
    Section {
        name: "profile",
        prefix: "qol_tray::features::profile",
    },
    Section {
        name: "runtime",
        prefix: "qol_tray::runtime",
    },
    Section {
        name: "shortcuts",
        prefix: "qol_tray::shortcuts",
    },
    Section {
        name: "task-runner",
        prefix: "qol_tray::features::task_runner",
    },
    Section {
        name: "tray",
        prefix: "qol_tray::tray",
    },
    Section {
        name: "updates",
        prefix: "qol_tray::updates",
    },
    Section {
        name: "features",
        prefix: "qol_tray::features",
    },
];

const CATCH_ALL: &str = "core";

pub fn is_valid_core_section(section: &str) -> bool {
    CORE_LOG_SECTION_IDS.contains(&section)
}

pub(super) struct CoreLogFilter {
    controls: CoreControlsHandle,
}

impl CoreLogFilter {
    pub(super) fn new(controls: CoreControlsHandle) -> Self {
        Self { controls }
    }
}

impl<S> tracing_subscriber::layer::Filter<S> for CoreLogFilter
where
    S: tracing::Subscriber,
{
    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        !is_target_muted(&self.controls, metadata.target())
    }

    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _cx: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        let message = event_message(event);
        !is_suppressed(&self.controls, event.metadata().target(), &message)
    }
}

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
    any_control_matches(controls, target, |control| {
        control_suppresses(control, message)
    })
}

fn is_target_muted(controls: &CoreControlsHandle, target: &str) -> bool {
    any_control_matches(controls, target, control_muted)
}

fn any_control_matches(
    controls: &CoreControlsHandle,
    target: &str,
    predicate: impl Fn(Option<&LogControl>) -> bool,
) -> bool {
    let Some(primary) = section_for_target(target) else {
        return false;
    };
    let controls = controls.read().unwrap_or_else(|e| e.into_inner());
    if predicate(controls.get(primary)) {
        return true;
    }
    if primary == CATCH_ALL {
        return false;
    }
    predicate(controls.get(CATCH_ALL))
}

fn control_suppresses(control: Option<&LogControl>, message: &str) -> bool {
    let Some(control) = control else {
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

fn control_muted(control: Option<&LogControl>) -> bool {
    control.map(|control| control.muted).unwrap_or(false)
}

fn event_message(event: &tracing::Event<'_>) -> String {
    let mut visitor = MessageVisitor::default();
    event.record(&mut visitor);
    visitor.message
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(entries: &[(&str, LogControl)]) -> CoreControlsHandle {
        Arc::new(RwLock::new(
            entries
                .iter()
                .map(|(key, control)| ((*key).to_string(), control.clone()))
                .collect(),
        ))
    }

    #[test]
    fn routes_targets_to_specific_sections() {
        let cases = [
            ("qol_tray::features::auth::health", Some("auth")),
            ("qol_tray::features::github_auth::service", Some("auth")),
            ("qol_tray::features::profile::sync", Some("profile")),
            (
                "qol_tray::features::plugin_store::server",
                Some("plugin-store"),
            ),
            (
                "qol_tray::features::task_runner::handlers",
                Some("task-runner"),
            ),
            ("qol_tray::shortcuts::executor", Some("shortcuts")),
            ("qol_tray::hotkeys::listener", Some("hotkeys")),
            ("qol_tray::desktop_state::platform", Some("desktop-state")),
            ("qol_tray::commands", Some("core")),
            ("other_crate::module", None),
        ];
        for (target, expected) in cases {
            assert_eq!(section_for_target(target), expected, "target={target}");
        }
    }

    #[test]
    fn core_catch_all_still_suppresses_specific_sections() {
        let controls = controls(&[(
            "core",
            LogControl {
                muted: true,
                suppress_patterns: vec![],
            },
        )]);

        assert!(is_suppressed(
            &controls,
            "qol_tray::shortcuts::executor",
            "run"
        ));
    }

    #[test]
    fn specific_section_patterns_suppress_matching_messages() {
        let controls = controls(&[(
            "shortcuts",
            LogControl {
                muted: false,
                suppress_patterns: vec!["skip me".to_string()],
            },
        )]);

        assert!(is_suppressed(
            &controls,
            "qol_tray::shortcuts::executor",
            "please skip me"
        ));
        assert!(!is_suppressed(
            &controls,
            "qol_tray::shortcuts::executor",
            "keep me"
        ));
    }
}
