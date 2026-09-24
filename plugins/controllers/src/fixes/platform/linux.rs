use super::super::state::SystemPaths;
use super::FixPlatform;
use anyhow::Result;
use std::path::{Path, PathBuf};

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
        qol_host_fixes::elevation::available()
    }

    fn apply(conf: &str, writes: &[(String, String)]) -> Result<()> {
        let script = r#"set -e
printf '%s' "$1" > /etc/modprobe.d/qol-controllers.conf
shift
while [ "$#" -ge 2 ]; do
  if [ -e "$1" ]; then printf '%s' "$2" > "$1"; fi
  shift 2
done"#;
        let mut args = vec![conf.to_string()];
        for (path, value) in writes {
            args.push(path.clone());
            args.push(value.clone());
        }
        qol_host_fixes::elevation::run_privileged("qol-controllers", script, &args)
    }

    fn hidraw_guard_path() -> Option<PathBuf> {
        Some(PathBuf::from(
            "/etc/udev/rules.d/72-qol-controllers-hidraw.rules",
        ))
    }

    fn install_hidraw_guard(path: &Path, content: &str) -> Result<()> {
        let script = r#"set -e
printf '%s' "$1" > "$2"
udevadm control --reload"#;
        let args = [content.to_string(), path.display().to_string()];
        qol_host_fixes::elevation::run_privileged("qol-controllers", script, &args)
    }
}
