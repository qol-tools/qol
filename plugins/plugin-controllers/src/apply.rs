use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::fixes::FixTarget;
use crate::state::desired_quirk;

fn quirks_by_driver(targets: &[FixTarget]) -> BTreeMap<&'static str, Vec<String>> {
    let mut map: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for target in targets {
        let quirks = map.entry(target.entry.module).or_default();
        let quirk = desired_quirk(target);
        if !quirks.contains(&quirk) {
            quirks.push(quirk);
        }
    }
    for quirks in map.values_mut() {
        quirks.sort();
    }
    map
}

pub fn conf_contents(targets: &[FixTarget]) -> String {
    quirks_by_driver(targets)
        .iter()
        .map(|(driver, quirks)| format!("options {driver} quirks={}\n", quirks.join(",")))
        .collect()
}

pub fn sysfs_writes(targets: &[FixTarget]) -> Vec<(String, String)> {
    quirks_by_driver(targets)
        .iter()
        .map(|(driver, quirks)| {
            (
                format!("/sys/module/{driver}/parameters/quirks"),
                quirks.join(","),
            )
        })
        .collect()
}

pub fn apply(targets: &[FixTarget]) -> Result<()> {
    if targets.is_empty() {
        bail!("no known controllers connected");
    }
    let conf = conf_contents(targets);
    let script = r#"set -e
printf '%s' "$1" > /etc/modprobe.d/qol-controllers.conf
shift
while [ "$#" -ge 2 ]; do
  if [ -e "$1" ]; then printf '%s' "$2" > "$1"; fi
  shift 2
done"#;
    let mut command = Command::new("pkexec");
    command.args(["sh", "-c", script, "qol-controllers", &conf]);
    for (path, value) in sysfs_writes(targets) {
        command.arg(path).arg(value);
    }
    let status = command.status().context("failed to launch pkexec")?;
    if !status.success() {
        bail!("pkexec exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::{match_devices, DetectedDevice, FixTarget};

    fn targets() -> Vec<FixTarget> {
        let device = DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x028e,
            version: 0x0903,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
            sysfs_path: None,
            event_handler: None,
            driver: Some("xpadneo".into()),
            is_gamepad: true,
            has_force_feedback: true,
        };
        match_devices(&[device])
    }

    #[test]
    fn conf_contents_regenerates_whole_file() {
        let expected = "options hid_xpadneo quirks=06:71:10:20:26:b4:263\n";
        assert_eq!(conf_contents(&targets()), expected);
    }

    #[test]
    fn sysfs_writes_target_driver_param() {
        let writes = sysfs_writes(&targets());
        assert_eq!(
            writes,
            vec![(
                "/sys/module/hid_xpadneo/parameters/quirks".to_string(),
                "06:71:10:20:26:b4:263".to_string()
            )]
        );
    }
}
