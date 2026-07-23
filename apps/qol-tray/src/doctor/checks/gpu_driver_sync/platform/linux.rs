use super::super::{is_version_token, parse_proc_version};
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
