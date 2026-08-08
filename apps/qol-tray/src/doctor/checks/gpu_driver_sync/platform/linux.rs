use super::PendingUpdate;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const GUARD_PATTERNS: [&str; 5] = [
    "nvidia-driver",
    "nvidia-driver-*",
    "nvidia-kernel-*",
    "nvidia-dkms-*",
    "nvidia-headless-*",
];

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

pub(crate) fn guard_supported() -> bool {
    Command::new("apt-mark")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn package_patterns() -> Vec<String> {
    #[cfg(feature = "dev")]
    if let Some(pattern) = std::env::var_os("QOL_NVIDIA_PACKAGE_PATTERN") {
        return vec![pattern.to_string_lossy().into_owned()];
    }
    GUARD_PATTERNS.iter().map(|p| p.to_string()).collect()
}

fn validate_pattern(pattern: &str) -> Result<()> {
    if pattern.is_empty()
        || !pattern
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'*' | b'?' | b'+' | b'-' | b'.'))
    {
        bail!("refusing to run apt against an unsafe package pattern `{pattern}`");
    }
    Ok(())
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

fn matches_any(name: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_pattern(pattern, name))
}

fn installed_matching(patterns: &[String]) -> Vec<String> {
    let Ok(output) = Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev} ${Package}\n"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("ii "))
        .filter_map(|line| line.split_once(' ').map(|(_, name)| name.trim()))
        .filter(|name| matches_any(name, patterns))
        .map(str::to_string)
        .collect()
}

fn guard_state_path() -> Result<PathBuf> {
    Ok(crate::paths::shared_config_dir()?.join("nvidia-driver-guard-holds"))
}

fn read_guard_holds() -> Vec<String> {
    let Ok(path) = guard_state_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_guard_holds(&text)
}

fn parse_guard_holds(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn write_guard_holds(names: &[String]) -> Result<()> {
    let path = guard_state_path()?;
    let parent = path
        .parent()
        .context("guard hold state has no parent directory")?;
    std::fs::create_dir_all(parent).context("failed to create guard state directory")?;
    let text = if names.is_empty() {
        String::new()
    } else {
        names.join("\n") + "\n"
    };
    let tmp = parent.join(format!(
        ".nvidia-driver-guard-holds.tmp{}",
        std::process::id()
    ));
    std::fs::write(&tmp, text).context("failed to write guard hold state")?;
    std::fs::rename(&tmp, &path).context("failed to commit guard hold state")?;
    Ok(())
}

fn clear_guard_holds() -> Result<()> {
    let path = guard_state_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to remove guard hold state"),
    }
}

fn held_among(names: &[String], patterns: &[String]) -> Vec<String> {
    let Ok(output) = Command::new("apt-mark").arg("showhold").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let held_now: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| matches_any(line, patterns))
        .map(str::to_string)
        .collect();
    names
        .iter()
        .filter(|name| held_now.iter().any(|held| held == *name))
        .cloned()
        .collect()
}

pub(crate) fn guard_armed() -> bool {
    !held_nvidia_packages().is_empty()
}

pub(crate) fn held_nvidia_packages() -> Vec<String> {
    let recorded = read_guard_holds();
    if recorded.is_empty() {
        return Vec::new();
    }
    held_among(&recorded, &package_patterns())
}

pub(crate) fn pending_nvidia_updates() -> Vec<PendingUpdate> {
    let recorded = read_guard_holds();
    if recorded.is_empty() {
        return Vec::new();
    }
    let Ok(output) = Command::new("apt").args(["list", "--upgradable"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_upgradable(
        &String::from_utf8_lossy(&output.stdout),
        &package_patterns(),
    )
    .into_iter()
    .filter(|update| recorded.iter().any(|name| name == &update.name))
    .collect()
}

pub(crate) fn parse_upgradable(text: &str, patterns: &[String]) -> Vec<PendingUpdate> {
    text.lines()
        .filter(|line| !line.starts_with("Listing"))
        .filter_map(|line| {
            let (name, rest) = line.split_once('/')?;
            if !matches_any(name, patterns) {
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
    if !qol_host_fixes::elevation::available() {
        bail!("holding NVIDIA packages needs polkit (pkexec) for the privileged apt-mark");
    }
    let patterns = package_patterns();
    for pattern in &patterns {
        validate_pattern(pattern)?;
    }
    let targets = installed_matching(&patterns);
    if targets.is_empty() {
        bail!("no installed NVIDIA driver packages match the guard patterns");
    }
    if !targets.iter().all(|name| is_package_token(name)) {
        bail!("refusing to run apt against a malformed package name");
    }
    let joined = targets.join(" ");
    qol_host_fixes::elevation::run_privileged(
        "qol-nvidia-guard",
        &format!("apt-mark hold {joined}"),
        &[],
    )?;
    let held = held_among(&targets, &patterns);
    if held.is_empty() {
        bail!("apt-mark hold did not hold any of the matched packages");
    }
    write_guard_holds(&held)?;
    super::super::trace::hold(&held, "done", None);
    Ok(())
}

pub(crate) fn unhold_driver_packages() -> Result<()> {
    if !qol_host_fixes::elevation::available() {
        bail!("releasing NVIDIA package holds needs polkit (pkexec) for the privileged apt-mark");
    }
    let patterns = package_patterns();
    let recorded = read_guard_holds();
    if recorded.is_empty() {
        return Ok(());
    }
    if !recorded.iter().all(|name| is_package_token(name)) {
        bail!("refusing to run apt against a malformed guard state entry");
    }
    let still_held = held_among(&recorded, &patterns);
    if !still_held.is_empty() {
        let joined = still_held.join(" ");
        qol_host_fixes::elevation::run_privileged(
            "qol-nvidia-guard",
            &format!("apt-mark unhold {joined}"),
            &[],
        )?;
    }
    clear_guard_holds()?;
    super::super::trace::unhold(&still_held, "done", None);
    Ok(())
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
    match qol_host_fixes::elevation::run_privileged("qol-nvidia-guard-update", &script, &[]) {
        Ok(()) => {
            super::super::trace::update(packages, "done", None);
            Ok(())
        }
        Err(error) => {
            super::super::trace::update(packages, "error", Some(&error.to_string()));
            Err(error)
        }
    }
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
         `qol-tray-doctor fix --id gpu_driver_sync --apply-manual-fixes`, then reboot.",
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
    use super::{is_version_token, parse_guard_holds, parse_proc_version, validate_pattern};

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

    #[test]
    fn guard_hold_state_round_trips_lines() {
        let parsed = parse_guard_holds("nvidia-driver-560\nnvidia-kernel-560\n\n");
        assert_eq!(parsed, ["nvidia-driver-560", "nvidia-kernel-560"]);
        assert!(parse_guard_holds("").is_empty());
        assert!(parse_guard_holds("\n \n").is_empty());
    }

    #[test]
    fn pattern_validation_rejects_shell_metacharacters() {
        for pattern in ["*nvidia*", "nvidia-driver-*", "nvidia-driver", "nvidia?560"] {
            assert!(validate_pattern(pattern).is_ok(), "{pattern}");
        }
        for pattern in [
            "nvidia;rm -rf /",
            "$(id)",
            "nvidia`id`",
            "nvidia&apt-get",
            "nvidia|sh",
            "nvidia>foo",
            "",
        ] {
            assert!(validate_pattern(pattern).is_err(), "{pattern}");
        }
    }
}
