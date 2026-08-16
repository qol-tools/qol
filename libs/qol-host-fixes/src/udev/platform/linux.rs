use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const GRANT_SCRIPT: &str = r#"set -eu
tmp="$1.qol-$$"
trap 'rm -f -- "$tmp"' EXIT HUP INT TERM
printf '%s' "$2" > "$tmp"
chmod 0644 "$tmp"
dd if=/dev/null of="$tmp" conv=fsync,notrunc 2>/dev/null || sync -d "$tmp" 2>/dev/null || sync
mv -f -- "$tmp" "$1"
trap - EXIT HUP INT TERM
printf '%s' "$2" | cmp -s - "$1" || { echo "qol-udev: the rule did not apply at $1" >&2; exit 4; }
udevadm control --reload
udevadm trigger --subsystem-match=i2c-dev
"#;

const RESTORE_SCRIPT: &str = r#"set -eu
if [ -e "$1" ] && ! printf '%s' "$2" | cmp -s - "$1"; then
    echo "qol-udev: refusing to remove a modified rule: $1" >&2
    exit 3
fi
rm -f -- "$1"
udevadm control --reload

i2c_dir=${QOL_UDEV_I2C_DEV_DIR:-/dev}
seat_users=${QOL_UDEV_SEAT_USER:-}
if [ -z "$seat_users" ]; then
    seat_users=$(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $3}' | sort -u)
fi

acl_live=0
if [ -n "$seat_users" ]; then
    if ! command -v getfacl >/dev/null 2>&1; then
        echo "qol-udev: refusing to revoke without getfacl to verify uaccess ACLs" >&2
        exit 5
    fi
    for node in "$i2c_dir"/i2c-*; do
        [ -e "$node" ] || continue
        for user in $seat_users; do
            if getfacl -cp -- "$node" 2>/dev/null | grep -Fq "user:$user:"; then
                setfacl -x "u:$user" -- "$node" 2>/dev/null || true
            fi
        done
        for user in $seat_users; do
            if getfacl -cp -- "$node" 2>/dev/null | grep -Fq "user:$user:"; then
                acl_live=1
                echo "qol-udev: uaccess ACL is still live on $node for $user" >&2
            fi
        done
    done
fi
if [ "$acl_live" -eq 1 ]; then
    exit 5
fi
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
