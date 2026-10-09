pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod remap;

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::Result;
use qol_headless::CommandResult;

use super::ConfigInspection;

pub(crate) trait Remapper {
    fn swap_config(&self, config: remap::ResolvedConfig);
    fn idle(&self);
    fn stop(self);
}

pub(crate) fn serve<R: Remapper>(
    poll: Duration,
    load: impl Fn() -> remap::ResolvedConfig,
    start: impl FnOnce(remap::ResolvedConfig) -> Result<R>,
) -> Result<()> {
    let resolved = load();

    log::info!(
        "loaded {} char rules, {} key rules, {} mouse rules, {} scroll rules, {} excluded apps",
        resolved.char_rules.len(),
        resolved.key_rules.len(),
        resolved.mouse_rules.len(),
        resolved.scroll_rules.len(),
        resolved.excluded_apps.len(),
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let Some(started) = start_services_if_singleton(daemon::start_listener(tx), || {
        let current_key_rules = resolved.key_rules.clone();
        start(resolved).map(|remapper| (current_key_rules, remapper))
    }) else {
        if daemon::send_reload() {
            log::debug!("another instance running, sent reload");
        }
        return Ok(());
    };
    let (mut current_key_rules, remapper) = match started {
        Ok(started) => started,
        Err(error) => {
            daemon::cleanup();
            return Err(error);
        }
    };

    log::info!("daemon started");

    loop {
        let command = match rx.recv_timeout(poll) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                remapper.idle();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match command {
            daemon::Command::Reload => {
                let new_resolved = load();
                log::debug!(
                    "reloaded {} char rules, {} key rules, {} mouse rules, {} scroll rules",
                    new_resolved.char_rules.len(),
                    new_resolved.key_rules.len(),
                    new_resolved.mouse_rules.len(),
                    new_resolved.scroll_rules.len(),
                );
                for warning in remap::diff_key_rules(&current_key_rules, &new_resolved.key_rules) {
                    log::warn!("{warning}");
                }
                current_key_rules = new_resolved.key_rules.clone();
                remapper.swap_config(new_resolved);
            }
            daemon::Command::Kill => {
                log::info!("kill received, shutting down");
                break;
            }
            daemon::Command::Settings => {
                if let Err(error) = qol_apps::desktop_integration::open_plugin_settings_via_tray(
                    crate::cli::PLUGIN_ID,
                ) {
                    log::warn!("failed to open settings page: {error}");
                }
            }
        }
        remapper.idle();
    }

    remapper.stop();
    daemon::cleanup();
    Ok(())
}

fn start_services_if_singleton<T>(is_singleton: bool, start: impl FnOnce() -> T) -> Option<T> {
    is_singleton.then(start)
}

pub(crate) fn reload() -> CommandResult {
    action_result(daemon::send_reload(), "reload sent", "no daemon running")
}

pub(crate) fn kill() -> CommandResult {
    action_result(daemon::send_kill(), "kill sent", "no daemon running")
}

pub(crate) fn toggle() -> CommandResult {
    let enabled = !config::load_config().enabled;
    let mut stored = qol_runtime::plugin_config::load_json()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(fields) = stored.as_object_mut() else {
        return CommandResult::runtime_error("keyremap: stored config is not an object");
    };
    fields.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
    if !qol_runtime::plugin_config::save(&stored) {
        return CommandResult::runtime_error("keyremap: failed to persist the new remapping state");
    }
    let state = if enabled {
        "key remapping enabled"
    } else {
        "key remapping disabled"
    };
    action_result(daemon::send_reload(), state, "no daemon running")
}

pub(crate) fn inspection(
    platform_issues: impl FnOnce(&config::RemapConfig) -> Vec<String>,
) -> Result<ConfigInspection> {
    let inspected = config::inspect_config()?;
    let mut issues = remap::validation_issues(&inspected.config);
    issues.extend(platform_issues(&inspected.config));
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

fn action_result(sent: bool, success: &str, missing: &str) -> CommandResult {
    let message = if sent { success } else { missing };
    CommandResult::new("", format!("[keyremap] {message}\n"), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_start_only_when_singleton() {
        let mut started = false;
        let result = start_services_if_singleton(false, || {
            started = true;
            0
        });
        assert!(result.is_none());
        assert!(!started);

        let result = start_services_if_singleton(true, || {
            started = true;
            42
        });
        assert_eq!(result, Some(42));
        assert!(started);
    }
}
