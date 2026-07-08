use std::fmt;

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
    pub name: String,
    pub uniq: Option<String>,
    pub is_gamepad: bool,
}

impl DetectedDevice {
    pub fn transport(&self) -> &'static str {
        match self.bus {
            0x0005 => "Bluetooth",
            0x0003 => "USB",
            _ => "Other",
        }
    }
}

pub struct FixEntry {
    pub id: &'static str,
    pub summary: &'static str,
    pub driver: &'static str,
    pub bus: u16,
    pub vendor: u16,
    pub products: &'static [u16],
    pub name: &'static str,
    pub quirk_value: u16,
}

pub const FIXES: &[FixEntry] = &[FixEntry {
    id: "gulikit-xw-bt-rumble",
    summary: "Rumble never stops over Bluetooth",
    driver: "hid_xpadneo",
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
        {
            continue;
        }
        let mac = device.uniq.as_deref().and_then(Mac::parse)?;
        return Some(FixTarget { entry, mac });
    }
    None
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
    ) -> DetectedDevice {
        DetectedDevice {
            bus,
            vendor,
            product,
            name: name.into(),
            uniq: uniq.map(str::to_string),
            is_gamepad: true,
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
        );
        let gulikit_alt_pid = device(
            0x0005,
            0x045e,
            0x02e0,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
        );
        let usb_clone = device(
            0x0003,
            0x045e,
            0x028e,
            "GuliKit Controller XW",
            Some("06:71:10:20:26:b4"),
        );
        let no_mac = device(0x0005, 0x045e, 0x028e, "GuliKit Controller XW", None);
        let other = device(0x0005, 0x054c, 0x0ce6, "foo pad", Some("aa:bb:cc:dd:ee:ff"));

        let cases: [(&str, Vec<DetectedDevice>, usize); 4] = [
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
        ];
        for (label, devices, expected) in cases {
            let targets = match_devices(&devices);
            assert_eq!(targets.len(), expected, "case: {label}");
        }

        let targets = match_devices(&[gulikit]);
        assert_eq!(targets[0].entry.id, "gulikit-xw-bt-rumble");
        assert_eq!(targets[0].mac.to_string(), "06:71:10:20:26:b4");
    }
}
