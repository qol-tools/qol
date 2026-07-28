use crate::fixes::DetectedDevice;

pub fn parse_devices(text: &str) -> Vec<DetectedDevice> {
    text.split("\n\n").filter_map(parse_block).collect()
}

fn parse_block(block: &str) -> Option<DetectedDevice> {
    let mut ids: Option<(u16, u16, u16, u16)> = None;
    let mut name: Option<String> = None;
    let mut uniq: Option<String> = None;
    let mut sysfs_path: Option<String> = None;
    let mut event_handler: Option<String> = None;
    let mut is_gamepad = false;
    let mut has_force_feedback = false;
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("I: ") {
            ids = parse_ids(rest);
        } else if let Some(rest) = line.strip_prefix("N: Name=") {
            name = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("U: Uniq=") {
            let value = rest.trim();
            uniq = (!value.is_empty()).then(|| value.to_string());
        } else if let Some(rest) = line.strip_prefix("S: Sysfs=") {
            let value = rest.trim();
            sysfs_path = (!value.is_empty()).then(|| value.to_string());
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            for handler in rest.split_whitespace() {
                if handler.starts_with("js") {
                    is_gamepad = true;
                }
                if is_event_handler(handler) {
                    event_handler = Some(handler.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("B: FF=") {
            has_force_feedback = bitmap_has_bits(rest);
        }
    }
    let (bus, vendor, product, version) = ids?;
    Some(DetectedDevice {
        bus,
        vendor,
        product,
        version,
        name: name?,
        uniq,
        sysfs_path,
        event_handler,
        driver: None,
        is_gamepad,
        has_force_feedback,
    })
}

fn is_event_handler(handler: &str) -> bool {
    handler.strip_prefix("event").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
    })
}

fn parse_ids(rest: &str) -> Option<(u16, u16, u16, u16)> {
    let mut bus = None;
    let mut vendor = None;
    let mut product = None;
    let mut version = None;
    for field in rest.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let parsed = u16::from_str_radix(value, 16).ok();
        match key {
            "Bus" => bus = parsed,
            "Vendor" => vendor = parsed,
            "Product" => product = parsed,
            "Version" => version = parsed,
            _ => {}
        }
    }
    Some((bus?, vendor?, product?, version?))
}

fn bitmap_has_bits(bitmap: &str) -> bool {
    bitmap
        .split_whitespace()
        .filter_map(|word| u128::from_str_radix(word, 16).ok())
        .any(|word| word != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
I: Bus=0005 Vendor=045e Product=028e Version=1130
N: Name=\"GuliKit Controller XW\"
P: Phys=10:91:d1:28:d5:a6
S: Sysfs=/devices/pci0000:00/bluetooth/hci0/input/input39
U: Uniq=06:71:10:20:26:b4
H: Handlers=event27 js0
B: FF=0

I: Bus=0003 Vendor=28de Product=11ff Version=0001
N: Name=\"Microsoft X-Box 360 pad 0\"
S: Sysfs=/devices/virtual/input/input40
U: Uniq=
H: Handlers=event30 js1
B: FF=10000 0

I: Bus=0005 Vendor=045e Product=028e Version=1130
N: Name=\"GuliKit Controller XW Consumer Control\"
U: Uniq=06:71:10:20:26:b4
H: Handlers=event28
";

    #[test]
    fn parser_extracts_bus_ids_name_uniq_and_gamepad_flag() {
        let devices = parse_devices(SAMPLE);
        assert_eq!(devices.len(), 3, "expected three device blocks");
        let first = &devices[0];
        assert_eq!(first.bus, 0x0005);
        assert_eq!(first.vendor, 0x045e);
        assert_eq!(first.product, 0x028e);
        assert_eq!(first.version, 0x1130);
        assert_eq!(first.name, "GuliKit Controller XW");
        assert_eq!(first.uniq.as_deref(), Some("06:71:10:20:26:b4"));
        assert_eq!(
            first.sysfs_path.as_deref(),
            Some("/devices/pci0000:00/bluetooth/hci0/input/input39")
        );
        assert_eq!(first.event_handler.as_deref(), Some("event27"));
        assert!(first.is_gamepad, "js handler marks a gamepad");
        assert!(!first.has_force_feedback, "zero FF bitmap has no bits");
        let second = &devices[1];
        assert_eq!(second.uniq, None, "empty Uniq= must map to None");
        assert!(second.is_gamepad);
        assert!(second.has_force_feedback, "non-zero FF bitmap has bits");
        assert!(second.is_virtual(), "virtual sysfs path is retained");
        let third = &devices[2];
        assert!(!third.is_gamepad, "no js handler means not a gamepad");
    }

    #[test]
    fn parser_skips_malformed_blocks() {
        let cases = [
            ("no I line", "N: Name=\"foo\"\n", 0),
            (
                "garbage ids",
                "I: Bus=zz Vendor=045e Product=028e Version=1130\nN: Name=\"foo\"\n",
                0,
            ),
            (
                "missing name",
                "I: Bus=0005 Vendor=045e Product=028e Version=1130\n",
                0,
            ),
        ];
        for (label, text, expected) in cases {
            assert_eq!(parse_devices(text).len(), expected, "case: {label}");
        }
    }
}
