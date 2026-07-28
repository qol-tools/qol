use super::super::state::SystemPaths;
use super::FixPlatform;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(super) struct Platform;

impl FixPlatform for Platform {
    fn system_paths() -> SystemPaths {
        SystemPaths {
            modprobe_dir: Some(PathBuf::from("/etc/modprobe.d")),
            sys_module_dir: Some(PathBuf::from("/sys/module")),
        }
    }

    fn live_quirk_path(driver: &str) -> Option<String> {
        Some(format!("/sys/module/{driver}/parameters/quirks"))
    }

    fn authorization_available() -> bool {
        Command::new("pkexec")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn apply(conf: &str, writes: &[(String, String)]) -> Result<()> {
        let script = r#"set -e
printf '%s' "$1" > /etc/modprobe.d/qol-controllers.conf
shift
while [ "$#" -ge 2 ]; do
  if [ -e "$1" ]; then printf '%s' "$2" > "$1"; fi
  shift 2
done"#;
        let mut command = Command::new("pkexec");
        command.args(["sh", "-c", script, "qol-controllers", conf]);
        for (path, value) in writes {
            command.arg(path).arg(value);
        }
        let status = command.status().context("failed to launch pkexec")?;
        if !status.success() {
            bail!("pkexec exited with {status}");
        }
        Ok(())
    }
}
