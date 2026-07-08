use std::path::PathBuf;

use crate::fixes::FixTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixState {
    DriverMissing,
    Pending,
    LiveOnly,
    Applied,
}

pub struct SystemPaths {
    pub modprobe_dir: PathBuf,
    pub sys_module_dir: PathBuf,
}

impl SystemPaths {
    pub fn real() -> SystemPaths {
        SystemPaths {
            modprobe_dir: PathBuf::from("/etc/modprobe.d"),
            sys_module_dir: PathBuf::from("/sys/module"),
        }
    }
}

pub fn desired_quirk(target: &FixTarget) -> String {
    format!("{}:{}", target.mac, target.entry.quirk_value)
}

pub fn compute(paths: &SystemPaths, target: &FixTarget, driver_installed: bool) -> FixState {
    if !driver_installed {
        return FixState::DriverMissing;
    }
    let quirk = desired_quirk(target);
    if persisted(paths, target.entry.driver, &quirk) {
        return FixState::Applied;
    }
    if live(paths, target.entry.driver, &quirk) {
        return FixState::LiveOnly;
    }
    FixState::Pending
}

fn persisted(paths: &SystemPaths, driver: &str, quirk: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(&paths.modprobe_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("conf") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if contents
            .lines()
            .any(|line| options_line_has_quirk(line, driver, quirk))
        {
            return true;
        }
    }
    false
}

fn options_line_has_quirk(line: &str, driver: &str, quirk: &str) -> bool {
    let normalized = line.trim().replace('-', "_");
    normalized.starts_with("options")
        && normalized.contains(&driver.replace('-', "_"))
        && normalized.contains(quirk)
}

fn live(paths: &SystemPaths, driver: &str, quirk: &str) -> bool {
    let param = paths.sys_module_dir.join(driver).join("parameters/quirks");
    std::fs::read_to_string(param)
        .map(|value| value.contains(quirk))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::{match_devices, DetectedDevice, FixTarget};
    use std::fs;

    fn target() -> FixTarget {
        let device = DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x028e,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
        };
        match_devices(&[device]).remove(0)
    }

    #[test]
    fn desired_quirk_formats_mac_and_value() {
        assert_eq!(desired_quirk(&target()), "06:71:10:20:26:b4:263");
    }

    #[test]
    fn state_reflects_filesystem() {
        let cases = [
            ("nothing anywhere", None, None, true, FixState::Pending),
            (
                "driver missing wins",
                None,
                None,
                false,
                FixState::DriverMissing,
            ),
            (
                "persisted in any conf",
                Some("options hid_xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                true,
                FixState::Applied,
            ),
            (
                "persisted with dash driver name",
                Some("options hid-xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                true,
                FixState::Applied,
            ),
            (
                "live only",
                None,
                Some("06:71:10:20:26:b4:263"),
                true,
                FixState::LiveOnly,
            ),
            (
                "other mac does not count",
                Some("options hid_xpadneo quirks=aa:bb:cc:dd:ee:ff:263"),
                None,
                true,
                FixState::Pending,
            ),
        ];
        for (label, conf_line, sysfs_value, driver_installed, expected) in cases {
            let root = tempfile::tempdir().expect("tempdir");
            let modprobe_dir = root.path().join("modprobe.d");
            let sys_module_dir = root.path().join("module");
            fs::create_dir_all(&modprobe_dir).expect("mkdir modprobe");
            if let Some(line) = conf_line {
                fs::write(modprobe_dir.join("a.conf"), format!("{line}\n")).expect("conf");
            }
            if let Some(value) = sysfs_value {
                let params = sys_module_dir.join("hid_xpadneo/parameters");
                fs::create_dir_all(&params).expect("mkdir params");
                fs::write(params.join("quirks"), value).expect("quirks");
            }
            let paths = SystemPaths {
                modprobe_dir,
                sys_module_dir,
            };
            assert_eq!(
                compute(&paths, &target(), driver_installed),
                expected,
                "case: {label}"
            );
        }
    }
}
