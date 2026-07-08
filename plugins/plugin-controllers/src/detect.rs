use crate::fixes::DetectedDevice;

pub fn read_devices() -> Vec<DetectedDevice> {
    std::fs::read_to_string("/proc/bus/input/devices")
        .map(|text| parse_devices(&text))
        .unwrap_or_default()
}

pub fn parse_devices(text: &str) -> Vec<DetectedDevice> {
    text.split("\n\n").filter_map(parse_block).collect()
}

fn parse_block(block: &str) -> Option<DetectedDevice> {
    let mut ids: Option<(u16, u16, u16)> = None;
    let mut name: Option<String> = None;
    let mut uniq: Option<String> = None;
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("I: ") {
            ids = parse_ids(rest);
        } else if let Some(rest) = line.strip_prefix("N: Name=") {
            name = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("U: Uniq=") {
            let value = rest.trim();
            uniq = (!value.is_empty()).then(|| value.to_string());
        }
    }
    let (bus, vendor, product) = ids?;
    Some(DetectedDevice {
        bus,
        vendor,
        product,
        name: name?,
        uniq,
    })
}

fn parse_ids(rest: &str) -> Option<(u16, u16, u16)> {
    let mut bus = None;
    let mut vendor = None;
    let mut product = None;
    for field in rest.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let parsed = u16::from_str_radix(value, 16).ok();
        match key {
            "Bus" => bus = parsed,
            "Vendor" => vendor = parsed,
            "Product" => product = parsed,
            _ => {}
        }
    }
    Some((bus?, vendor?, product?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
I: Bus=0005 Vendor=045e Product=028e Version=1130
N: Name=\"GuliKit Controller XW\"
P: Phys=10:91:d1:28:d5:a6
U: Uniq=06:71:10:20:26:b4
H: Handlers=event27 js0

I: Bus=0003 Vendor=28de Product=11ff Version=0001
N: Name=\"Microsoft X-Box 360 pad 0\"
U: Uniq=
H: Handlers=event30 js1
";

    #[test]
    fn parser_extracts_bus_ids_name_and_uniq() {
        let devices = parse_devices(SAMPLE);
        assert_eq!(devices.len(), 2, "expected two device blocks");
        let first = &devices[0];
        assert_eq!(first.bus, 0x0005);
        assert_eq!(first.vendor, 0x045e);
        assert_eq!(first.product, 0x028e);
        assert_eq!(first.name, "GuliKit Controller XW");
        assert_eq!(first.uniq.as_deref(), Some("06:71:10:20:26:b4"));
        let second = &devices[1];
        assert_eq!(second.uniq, None, "empty Uniq= must map to None");
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
