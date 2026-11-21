pub(crate) mod remap;

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::Result;
use qol_headless::CommandResult;

use super::ConfigInspection;
use crate::config::{Inspection, RemapConfig};
use crate::control::{Command, Host};

pub(crate) trait Remapper {
    fn swap_config(&self, config: remap::ResolvedConfig, raw: &RemapConfig);
    fn idle(&self);
    fn stop(self);
}

pub(crate) fn serve<R: Remapper>(
    host: &Host,
    poll: Duration,
    adjust: fn(RemapConfig) -> RemapConfig,
    start: impl FnOnce(remap::ResolvedConfig, &RemapConfig) -> Result<R>,
) -> Result<()> {
    let load = || {
        let raw = adjust((host.read_config)());
        (remap::resolve(&raw), raw)
    };
    let (resolved, raw) = load();

    log::info!(
        "loaded {} char rules, {} key rules, {} mouse rules, {} scroll rules, {} excluded apps",
        resolved.char_rules.len(),
        resolved.key_rules.len(),
        resolved.mouse_rules.len(),
        resolved.scroll_rules.len(),
        resolved.excluded_apps.len(),
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let Some(started) = (host.listen)(tx).then(|| {
        let current_key_rules = resolved.key_rules.clone();
        start(resolved, &raw).map(|remapper| (current_key_rules, remapper))
    }) else {
        if (host.send_reload)() {
            log::debug!("another instance running, sent reload");
        }
        return Ok(());
    };
    let (mut current_key_rules, remapper) = match started {
        Ok(started) => started,
        Err(error) => {
            (host.cleanup)();
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
        if matches!(command, Command::Toggle) {
            if let Err(message) = (host.flip_enabled)() {
                log::warn!("{message}");
            }
        }
        match command {
            Command::Reload | Command::Toggle => {
                let (new_resolved, new_raw) = load();
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
                remapper.swap_config(new_resolved, &new_raw);
            }
            Command::Kill => {
                log::info!("kill received, shutting down");
                break;
            }
            Command::Settings => {
                if let Err(error) = (host.open_settings)() {
                    log::warn!("failed to open settings page: {error}");
                }
            }
        }
        remapper.idle();
    }

    remapper.stop();
    (host.cleanup)();
    Ok(())
}

pub(crate) fn reload(host: &Host) -> CommandResult {
    action_result((host.send_reload)(), "reload sent", "no daemon running")
}

pub(crate) fn kill(host: &Host) -> CommandResult {
    action_result((host.send_kill)(), "kill sent", "no daemon running")
}

pub(crate) fn toggle(host: &Host) -> CommandResult {
    let enabled = match (host.flip_enabled)() {
        Ok(enabled) => enabled,
        Err(message) => return CommandResult::runtime_error(message),
    };
    let state = if enabled {
        "key remapping enabled"
    } else {
        "key remapping disabled"
    };
    action_result((host.send_reload)(), state, "no daemon running")
}

pub(crate) fn inspection(
    inspected: &Inspection,
    platform_issues: impl FnOnce(&RemapConfig) -> Vec<String>,
) -> ConfigInspection {
    let mut issues = remap::validation_issues(&inspected.config);
    issues.extend(platform_issues(&inspected.config));
    ConfigInspection {
        source: inspected.source.is_some(),
        enabled: inspected.config.enabled,
        char_rules: inspected.config.char_rules.len(),
        char_swaps: inspected.config.char_swaps.len(),
        key_rules: inspected.config.key_rules.len(),
        mouse_rules: inspected.config.mouse_rules.len(),
        scroll_rules: inspected.config.scroll_rules.len(),
        issues,
    }
}

fn action_result(sent: bool, success: &str, missing: &str) -> CommandResult {
    let message = if sent { success } else { missing };
    CommandResult::new("", format!("[keyremap] {message}\n"), 0)
}
