use std::fmt;

pub mod apply;
mod platform;
pub mod state;

pub(crate) use platform::authorization_available;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mac([u8; 6]);

impl Mac {
    pub fn parse(input: &str) -> Option<Mac> {
        let mut bytes = [0u8; 6];
        let mut parts = input.split(':');
        for byte in &mut bytes {
            let part = parts.next()?;
            if part.len() != 2 {
                return None;
            }
            *byte = u8::from_str_radix(part, 16).ok()?;
        }
        match parts.next() {
            None => Some(Mac(bytes)),
            Some(_) => None,
        }
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

#[derive(Clone)]
pub struct DetectedDevice {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub name: String,
    pub uniq: Option<String>,
    pub sysfs_path: Option<String>,
    pub event_handler: Option<String>,
    pub driver: Option<String>,
    pub is_gamepad: bool,
    pub has_force_feedback: bool,
}

impl DetectedDevice {
    pub fn transport(&self) -> &'static str {
        if self.is_virtual() {
            return "Virtual";
        }
        match self.bus {
            0x0005 => "Bluetooth",
            0x0003 => "USB",
            _ => "Other",
        }
    }

    pub fn is_virtual(&self) -> bool {
        self.sysfs_path
            .as_deref()
            .is_some_and(|path| path.starts_with("/devices/virtual/"))
    }

    pub fn driver_label(&self) -> &str {
        if let Some(driver) = self.driver.as_deref() {
            return driver;
        }
        if self.is_virtual() {
            return "userspace";
        }
        "unknown"
    }

    pub fn version_label(&self) -> String {
        format!("{:04x}", self.version)
    }
}

pub struct FixEntry {
    pub id: &'static str,
    pub summary: &'static str,
    pub module: &'static str,
    pub bound_driver: &'static str,
    pub bus: u16,
    pub vendor: u16,
    pub products: &'static [u16],
    pub name: &'static str,
    pub quirk_value: u16,
}

pub const FIXES: &[FixEntry] = &[FixEntry {
    id: "gulikit-xw-bt-rumble",
    summary: "Optional xpadneo rumble workaround",
    module: "hid_xpadneo",
    bound_driver: "xpadneo",
    bus: 0x0005,
    vendor: 0x045e,
    products: &[0x02e0, 0x028e],
    name: "GuliKit Controller XW",
    quirk_value: 263,
}];

#[derive(Clone)]
pub struct FixTarget {
    pub entry: &'static FixEntry,
    pub mac: Mac,
}

pub fn match_device(device: &DetectedDevice) -> Option<FixTarget> {
    for entry in FIXES {
        if device.bus != entry.bus
            || device.vendor != entry.vendor
            || !entry.products.contains(&device.product)
            || device.name != entry.name
            || !device
                .driver
                .as_deref()
                .is_some_and(|driver| driver_names_match(driver, entry.bound_driver))
        {
            continue;
        }
        let mac = device.uniq.as_deref().and_then(Mac::parse)?;
        return Some(FixTarget { entry, mac });
    }
    None
}

fn driver_names_match(actual: &str, expected: &str) -> bool {
    actual.replace('-', "_") == expected.replace('-', "_")
}

pub fn match_devices(devices: &[DetectedDevice]) -> Vec<FixTarget> {
    let mut targets: Vec<FixTarget> = Vec::new();
    for device in devices {
        let Some(target) = match_device(device) else {
            continue;
        };
        let duplicate = targets
            .iter()
            .any(|existing| existing.entry.id == target.entry.id && existing.mac == target.mac);
        if !duplicate {
            targets.push(target);
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_parsing_accepts_only_six_hex_pairs() {
        let cases = [
            ("06:71:10:20:26:b4", Some("06:71:10:20:26:b4")),
            ("06:71:10:20:26:B4", Some("06:71:10:20:26:b4")),
            ("06:71:10:20:26", None),
            ("06:71:10:20:26:b4:ff", None),
            ("gg:71:10:20:26:b4", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                Mac::parse(input).map(|m| m.to_string()),
                expected.map(str::to_string),
                "input: {input}"
            );
        }
    }

    fn device(
        bus: u16,
        vendor: u16,
        product: u16,
        name: &str,
        uniq: Option<&str>,
        driver: Option<&str>,
    ) -> DetectedDevice {
        DetectedDevice {
            bus,
            vendor,
            product,
            version: 0x0903,
            name: name.into(),
            uniq: uniq.map(str::to_string),
            sysfs_path: None,
            event_handler: None,
            driver: driver.map(str::to_string),
            is_gamepad: true,
            has_force_feedback: false,
        }
    }

    #[test]
    fn matching_selects_known_pads_and_dedupes() {
        let gulikit = device(
            0x0005,
            0x045e,
            0x028e,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
            Some("xpadneo"),
        );
        let gulikit_alt_pid = device(
            0x0005,
            0x045e,
            0x02e0,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
            Some("xpadneo"),
        );
        let usb_clone = device(
            0x0003,
            0x045e,
            0x028e,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
            Some("xpadneo"),
        );
        let no_mac = device(
            0x0005,
            0x045e,
            0x028e,
            "GuliKit Controller XW",
            None,
            Some("xpadneo"),
        );
        let other = device(
            0x0005,
            0x054c,
            0x0ce6,
            "foo pad",
            Some("aa:bb:cc:dd:ee:ff"),
            Some("xpadneo"),
        );
        let native_driver = device(
            0x0005,
            0x045e,
            0x02e0,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
            Some("hid-generic"),
        );

        let cases: [(&str, Vec<DetectedDevice>, usize); 5] = [
            ("single match", vec![gulikit.clone()], 1),
            (
                "same pad twice dedupes",
                vec![gulikit.clone(), gulikit_alt_pid],
                1,
            ),
            (
                "usb transport and unknown pad ignored",
                vec![usb_clone, other],
                0,
            ),
            ("missing mac ignored", vec![no_mac], 0),
            (
                "fix for optional driver ignores native driver",
                vec![native_driver],
                0,
            ),
        ];
        for (label, devices, expected) in cases {
            let targets = match_devices(&devices);
            assert_eq!(targets.len(), expected, "case: {label}");
        }

        let targets = match_devices(&[gulikit]);
        assert_eq!(targets[0].entry.id, "gulikit-xw-bt-rumble");
        assert_eq!(targets[0].mac.to_string(), "06:71:10:20:26:b4");
    }

    #[test]
    fn device_labels_virtual_and_physical_paths() {
        let mut virtual_pad = device(0x0003, 0x28de, 0x11ff, "Virtual pad", None, None);
        virtual_pad.sysfs_path = Some("/devices/virtual/input/input40".into());
        assert_eq!(virtual_pad.transport(), "Virtual");
        assert_eq!(virtual_pad.driver_label(), "userspace");

        let physical_pad = device(
            0x0005,
            0x045e,
            0x02e0,
            "Physical pad",
            None,
            Some("hid-generic"),
        );
        assert_eq!(physical_pad.transport(), "Bluetooth");
        assert_eq!(physical_pad.driver_label(), "hid-generic");
        assert_eq!(physical_pad.version_label(), "0903");
    }
}
