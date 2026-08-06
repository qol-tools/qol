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
