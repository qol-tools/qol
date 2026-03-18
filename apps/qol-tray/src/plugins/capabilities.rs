use crate::plugins::manifest::Capabilities;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Command;

struct CapabilityRule {
    name: &'static str,
    os: &'static str,
    is_required: fn(&Capabilities) -> bool,
    check: fn() -> bool,
    fix: fn() -> Result<()>,
}

const REGISTRY: &[CapabilityRule] = &[CapabilityRule {
    name: "serial",
    os: "linux",
    is_required: |c| c.serial,
    check: check_serial_linux,
    fix: fix_serial_linux,
}];

pub fn ensure_capabilities(capabilities_list: &[&Capabilities]) -> HashMap<&'static str, bool> {
    let mut results = HashMap::new();

    for rule in REGISTRY {
        if rule.os != std::env::consts::OS {
            continue;
        }
        if results.contains_key(rule.name) {
            continue;
        }
        let needed = capabilities_list.iter().any(|c| (rule.is_required)(c));
        if !needed {
            results.insert(rule.name, true);
            continue;
        }
        if (rule.check)() {
            results.insert(rule.name, true);
            continue;
        }
        log::info!("capability '{}' not met, attempting fix", rule.name);
        let fixed = (rule.fix)().is_ok() && (rule.check)();
        if fixed {
            log::info!("capability '{}' fixed", rule.name);
        } else {
            log::warn!("capability '{}' could not be fixed", rule.name);
        }
        results.insert(rule.name, fixed);
    }

    results
}

pub fn capabilities_met(
    capabilities: &Capabilities,
    results: &HashMap<&'static str, bool>,
) -> bool {
    REGISTRY.iter().all(|rule| {
        if rule.os != std::env::consts::OS {
            return true;
        }
        if !(rule.is_required)(capabilities) {
            return true;
        }
        results.get(rule.name).copied().unwrap_or(false)
    })
}

fn check_serial_linux() -> bool {
    let output = Command::new("id").arg("-nG").output();
    let Ok(output) = output else { return false };
    let groups = String::from_utf8_lossy(&output.stdout);
    groups.split_whitespace().any(|g| g == "dialout")
}

fn fix_serial_linux() -> Result<()> {
    let user = std::env::var("USER").context("USER env var not set")?;

    Command::new("pkexec")
        .args(["usermod", "-aG", "dialout", &user])
        .status()
        .context("pkexec usermod failed")?;

    let devices: Vec<_> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("ttyUSB") || n.starts_with("ttyACM"))
        })
        .collect();

    for device in devices {
        let _ = Command::new("pkexec")
            .args(["chmod", "660", &device.path().to_string_lossy()])
            .status();
    }

    Ok(())
}
