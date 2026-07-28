use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::state::desired_quirk;
use super::FixTarget;

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
        .filter_map(|(driver, quirks)| {
            super::platform::live_quirk_path(driver).map(|path| (path, quirks.join(",")))
        })
        .collect()
}

pub fn apply(targets: &[FixTarget]) -> Result<()> {
    if targets.is_empty() {
        bail!("no known controllers connected");
    }
    let conf = conf_contents(targets);
    super::platform::apply(&conf, &sysfs_writes(targets))
}

#[cfg(test)]
mod tests {
    use super::super::{match_devices, DetectedDevice, FixTarget};
    use super::*;

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
