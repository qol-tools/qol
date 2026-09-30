mod app;
mod app_tracker;
mod hid_helper;
mod input;
mod layout;
mod secure_input;
mod tap;
mod virtual_hid;

use anyhow::Result;
use qol_headless::CommandResult;

use super::{
    ConfigInspection, DriverState, HelperState, LayoutGap, PlatformAdapter, Probe,
    SecureInputHolder, TrustStatus,
};

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

    fn hid_helper(&self) -> Result<CommandResult> {
        hid_helper::run()?;
        Ok(CommandResult::success(""))
    }

    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::install()))
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::uninstall()))
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

    fn virtual_hid_driver(&self) -> Probe<DriverState> {
        virtual_hid::driver::driver_state()
    }

    fn virtual_hid_daemon(&self) -> Probe<bool> {
        virtual_hid::driver::daemon_running()
    }

    fn hid_helper_state(&self) -> Probe<HelperState> {
        hid_helper::query_status()
    }

    fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
        Probe::Known(secure_input::holder())
    }

    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
        let snapshot = match layout::LayoutSnapshot::read_current() {
            Ok(snapshot) => snapshot,
            Err(error) => return Probe::Unknown(error.to_string()),
        };
        let resolved = app::remap::resolve(&app::config::load_config());
        let gaps = app::remap::character_targets(&resolved)
            .into_iter()
            .flat_map(|(rule, text)| {
                layout::missing_characters(&snapshot.table, [text.as_str()])
                    .into_iter()
                    .map(move |character| LayoutGap {
                        rule: rule.clone(),
                        character,
                    })
            })
            .collect();
        Probe::Known(gaps)
    }
}

fn action_result(sent: bool, success: &str, missing: &str) -> CommandResult {
    let message = if sent { success } else { missing };
    CommandResult::new("", format!("[keyremap] {message}\n"), 0)
}

fn summary_result(result: Result<String>) -> CommandResult {
    match result {
        Ok(summary) => CommandResult::success(format!("{summary}\n")),
        Err(error) => CommandResult::runtime_error(format!("keyremap: {error:#}")),
    }
}
