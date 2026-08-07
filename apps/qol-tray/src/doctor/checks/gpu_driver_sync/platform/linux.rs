use super::PendingUpdate;
use anyhow::{bail, Result};
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn watch_supported() -> bool {
    true
}

pub(crate) fn loaded_version() -> Option<String> {
    let text = std::fs::read_to_string(proc_version_path()).ok()?;
    parse_proc_version(&text)
}

fn proc_version_path() -> PathBuf {
    #[cfg(feature = "dev")]
    if let Some(path) = std::env::var_os("QOL_NVIDIA_PROC_VERSION") {
        return PathBuf::from(path);
    }
    PathBuf::from("/proc/driver/nvidia/version")
}

pub(crate) fn on_disk_version() -> Option<String> {
    ["modinfo", "/usr/sbin/modinfo", "/sbin/modinfo"]
        .iter()
        .find_map(|binary| {
            let output = Command::new(binary)
                .args(["-F", "version", "nvidia"])
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .filter(|version| is_version_token(version))
}

fn package_pattern() -> String {
    #[cfg(feature = "dev")]
    if let Some(pattern) = std::env::var_os("QOL_NVIDIA_PACKAGE_PATTERN") {
        return pattern.to_string_lossy().into_owned();
    }
    "*nvidia*".to_string()
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    let core = pattern.trim_matches('*');
    if core.is_empty() {
        return false;
    }
    match (pattern.starts_with('*'), pattern.ends_with('*')) {
        (true, true) => name.contains(core),
        (false, true) => name.starts_with(core),
        (true, false) => name.ends_with(core),
        (false, false) => name == core,
    }
}

pub(crate) fn guard_armed() -> bool {
    !held_nvidia_packages().is_empty()
}

pub(crate) fn held_nvidia_packages() -> Vec<String> {
    let pattern = package_pattern();
    let Ok(output) = Command::new("apt-mark").arg("showhold").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| matches_pattern(&pattern, line))
        .map(str::to_string)
        .collect()
}

pub(crate) fn pending_nvidia_updates() -> Vec<PendingUpdate> {
    let Ok(output) = Command::new("apt").args(["list", "--upgradable"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_upgradable(&String::from_utf8_lossy(&output.stdout), &package_pattern())
}

pub(crate) fn parse_upgradable(text: &str, pattern: &str) -> Vec<PendingUpdate> {
    text.lines()
        .filter(|line| !line.starts_with("Listing"))
        .filter_map(|line| {
            let (name, rest) = line.split_once('/')?;
            if !matches_pattern(pattern, name) {
                return None;
            }
            let new_version = rest.split_whitespace().nth(1)?;
            Some(PendingUpdate {
                name: name.to_string(),
                new_version: new_version.to_string(),
            })
        })
        .collect()
}

pub(crate) fn hold_driver_packages() -> Result<()> {
    run_apt_mark("hold")
}

pub(crate) fn unhold_driver_packages() -> Result<()> {
    run_apt_mark("unhold")
}

fn run_apt_mark(verb: &str) -> Result<()> {
    if !qol_host_fixes::elevation::available() {
        bail!("holding NVIDIA packages needs polkit (pkexec) for the privileged apt-mark");
    }
    let pattern = package_pattern();
    qol_host_fixes::elevation::run_privileged(
        "qol-nvidia-guard",
        &format!("apt-mark {verb} '{pattern}'"),
        &[],
    )
}

pub(crate) fn apply_held_update(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        bail!("no held NVIDIA updates to apply");
    }
    if !packages.iter().all(|name| is_package_token(name)) {
        bail!("refusing to run apt against a malformed package name");
    }
    if !qol_host_fixes::elevation::available() {
        bail!("applying a driver update needs polkit (pkexec)");
    }
    let joined = packages.join(" ");
    let script = format!(
        "rehold() {{ apt-mark hold {joined} || true; }}; trap rehold EXIT; \
         apt-mark unhold {joined} && apt-get install --only-upgrade -y {joined}"
    );
    qol_host_fixes::elevation::run_privileged("qol-nvidia-guard-update", &script, &[])
}

fn is_package_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'+' | b':' | b'_'))
}

pub(crate) fn notify_held_updates(packages: &[String]) {
    let message = format!(
        "NVIDIA driver update held by the qol guard: {}. Apply with \
         `qol-tray doctor fix --id gpu_driver_sync --apply-manual-fixes`, then reboot.",
        packages.join(", ")
    );
    let _ = Command::new("notify-send")
        .args(["--icon=qol-tray", "--urgency=normal", "QoL Tray", &message])
        .status();
}

pub(crate) fn notify_mismatch(loaded: &str, on_disk: &str) {
    let message = format!(
        "NVIDIA driver updated on disk ({on_disk}) while the kernel still runs {loaded}. \
         New OpenGL apps will fail to start until a reboot loads the matching module."
    );
    let _ = Command::new("notify-send")
        .args([
            "--icon=qol-tray",
            "--urgency=critical",
            "QoL Tray",
            &message,
        ])
        .status();
}

fn parse_proc_version(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.starts_with("NVRM version:"))?
        .split_whitespace()
        .find(|token| is_version_token(token))
        .map(str::to_string)
}

fn is_version_token(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{is_version_token, parse_proc_version};

    #[test]
    fn proc_version_parsing_extracts_module_version() {
        let cases = [
            (
                "NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.159.02  Wed May 14 21:38:31 UTC 2025\nGCC version:  gcc version 13.3.0",
                Some("580.159.02"),
            ),
            (
                "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.65.06  Release Build",
                Some("580.65.06"),
            ),
            ("GCC version:  gcc version 13.3.0", None),
            ("NVRM version: NVIDIA UNIX Kernel Module", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_proc_version(input).as_deref(),
                expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn version_token_accepts_dotted_numerics_only() {
        let cases = [
            ("580.159.02", true),
            ("580.65", true),
            ("580", false),
            ("x86_64", false),
            ("580.159.", false),
            ("Module", false),
            ("580.abc", false),
        ];
        for (input, expected) in cases {
            assert_eq!(is_version_token(input), expected, "input: {input:?}");
        }
    }
}
