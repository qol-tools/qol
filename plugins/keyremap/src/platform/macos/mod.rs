mod app;
mod app_tracker;
mod tap;

use anyhow::Result;
use qol_headless::CommandResult;

use super::{ConfigInspection, PlatformAdapter, TrustStatus};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn supported(&self) -> bool {
        true
    }

    fn launch(&self) -> Result<CommandResult> {
        app::run();
        Ok(CommandResult::success(""))
    }

    fn reload(&self) -> Result<CommandResult> {
        Ok(action_result(
            app::daemon::send_reload(),
            "reload sent",
            "no daemon running",
        ))
    }

    fn toggle(&self) -> Result<CommandResult> {
        let enabled = !app::config::load_config().enabled;
        let mut stored = qol_runtime::plugin_config::load_json()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(fields) = stored.as_object_mut() else {
            return Ok(CommandResult::runtime_error(
                "keyremap: stored config is not an object",
            ));
        };
        fields.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
        if !qol_runtime::plugin_config::save(&stored) {
            return Ok(CommandResult::runtime_error(
                "keyremap: failed to persist the new remapping state",
            ));
        }
        let state = if enabled {
            "key remapping enabled"
        } else {
            "key remapping disabled"
        };
        Ok(action_result(
            app::daemon::send_reload(),
            state,
            "no daemon running",
        ))
    }

    fn kill(&self) -> Result<CommandResult> {
        Ok(action_result(
            app::daemon::send_kill(),
            "kill sent",
            "no daemon running",
        ))
    }

    fn inspect_config(&self) -> Result<ConfigInspection> {
        let inspected = app::config::inspect_config()?;
        let issues = app::remap::validation_issues(&inspected.config);
        Ok(ConfigInspection {
            source: inspected.source.is_some(),
            enabled: inspected.config.enabled,
            char_rules: inspected.config.char_rules.len(),
            char_swaps: inspected.config.char_swaps.len(),
            key_rules: inspected.config.key_rules.len(),
            mouse_rules: inspected.config.mouse_rules.len(),
            scroll_rules: inspected.config.scroll_rules.len(),
            issues,
        })
    }

    fn trust_status(&self) -> TrustStatus {
        TrustStatus::from_trusted(tap::accessibility_trusted())
    }
}

fn action_result(sent: bool, success: &str, missing: &str) -> CommandResult {
    let message = if sent { success } else { missing };
    CommandResult::new("", format!("[keyremap] {message}\n"), 0)
}
