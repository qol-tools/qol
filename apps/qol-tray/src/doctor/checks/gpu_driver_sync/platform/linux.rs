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
