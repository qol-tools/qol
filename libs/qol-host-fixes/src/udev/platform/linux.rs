use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const GRANT_SCRIPT: &str = r#"set -eu
printf '%s' "$2" > "$1"
udevadm control --reload
udevadm trigger
"#;

const RESTORE_SCRIPT: &str = r#"set -eu
if [ -e "$1" ] && ! printf '%s' "$2" | cmp -s - "$1"; then
    echo "qol-udev: refusing to remove a modified rule: $1" >&2
    exit 3
fi
rm -f -- "$1"
udevadm control --reload
"#;

pub(crate) fn rules_dir() -> PathBuf {
    #[cfg(any(test, feature = "sandbox"))]
    if let Some(dir) = std::env::var_os("QOL_UDEV_RULES_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(super::RULES_DIR)
}

pub(crate) fn grant(rule_path: &Path, rule_content: &str) -> Result<()> {
    let args = [rule_path.display().to_string(), rule_content.to_string()];
    run_root("qol-udev-uaccess-grant", GRANT_SCRIPT, &args)
        .with_context(|| format!("failed to write the uaccess rule {}", rule_path.display()))
}

pub(crate) fn restore_rule(rule_path: &Path, rule_content: &str) -> Result<()> {
    let args = [rule_path.display().to_string(), rule_content.to_string()];
    run_root("qol-udev-uaccess-restore", RESTORE_SCRIPT, &args)
        .with_context(|| format!("failed to remove the uaccess rule {}", rule_path.display()))
}

fn run_root(label: &str, script: &str, args: &[String]) -> Result<()> {
    #[cfg(any(test, feature = "sandbox"))]
    {
        let _ = label;
        run_sh(script, args)
    }
    #[cfg(not(any(test, feature = "sandbox")))]
    {
        if crate::privilege::is_elevated() {
            run_sh(script, args)
        } else {
            crate::elevation::run_privileged(label, script, args)
        }
    }
}

fn run_sh(script: &str, args: &[String]) -> Result<()> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("qol-udev-uaccess")
        .args(args)
        .output()
        .context("failed to launch the udev uaccess script")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        bail!("udev uaccess script exited {}", output.status);
    }
    bail!("udev uaccess script failed: {stderr}")
}
