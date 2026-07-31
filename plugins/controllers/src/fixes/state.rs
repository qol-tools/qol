use std::path::PathBuf;

use super::FixTarget;

pub use qol_host_fixes::FixState;

pub struct SystemPaths {
    pub modprobe_dir: Option<PathBuf>,
    pub sys_module_dir: Option<PathBuf>,
}

impl SystemPaths {
    pub fn real() -> SystemPaths {
        super::platform::system_paths()
    }
}

pub fn desired_quirk(target: &FixTarget) -> String {
    format!("{}:{}", target.mac, target.entry.quirk_value)
}

pub fn compute(paths: &SystemPaths, target: &FixTarget) -> FixState {
    let quirk = desired_quirk(target);
    if persisted(paths, target.entry.module, &quirk) {
        return FixState::Applied;
    }
    if live(paths, target.entry.module, &quirk) {
        return FixState::LiveOnly;
    }
    FixState::Pending
}

fn persisted(paths: &SystemPaths, driver: &str, quirk: &str) -> bool {
    let Some(modprobe_dir) = paths.modprobe_dir.as_ref() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(modprobe_dir) else {
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
    let Some(sys_module_dir) = paths.sys_module_dir.as_ref() else {
        return false;
    };
    let param = sys_module_dir.join(driver).join("parameters/quirks");
    std::fs::read_to_string(param)
        .map(|value| value.contains(quirk))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::super::{match_devices, DetectedDevice, FixTarget};
    use super::*;
    use std::fs;

    fn target() -> FixTarget {
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
        match_devices(&[device]).remove(0)
    }

    #[test]
    fn desired_quirk_formats_mac_and_value() {
        assert_eq!(desired_quirk(&target()), "06:71:10:20:26:b4:263");
    }

    #[test]
    fn state_reflects_filesystem() {
        let cases = [
            ("nothing anywhere", None, None, FixState::Pending),
            (
                "persisted in any conf",
                Some("options hid_xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                FixState::Applied,
            ),
            (
                "persisted with dash driver name",
                Some("options hid-xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                FixState::Applied,
            ),
            (
                "live only",
                None,
                Some("06:71:10:20:26:b4:263"),
                FixState::LiveOnly,
            ),
            (
                "other mac does not count",
                Some("options hid_xpadneo quirks=aa:bb:cc:dd:ee:ff:263"),
                None,
                FixState::Pending,
            ),
        ];
        for (label, conf_line, sysfs_value, expected) in cases {
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
                modprobe_dir: Some(modprobe_dir),
                sys_module_dir: Some(sys_module_dir),
            };
            assert_eq!(compute(&paths, &target()), expected, "case: {label}");
        }
    }
}
