use serialport::SerialPortInfo;

const SONOFF_VID: u16 = 0x10C4;
const SONOFF_PID: u16 = 0xEA60;
const PRIMARY_USB_IDENTIFIERS: &[&str] = &[
    "sonoff",
    "itead",
    "zbdongle",
    "cc2531",
    "cc2538",
    "cc2652",
    "cc2652r",
    "cc2652p",
    "cc1352",
    "cc1352p",
    "slaesh",
    "electrolama",
    "zzh",
    "tube",
];
const SECONDARY_USB_IDENTIFIERS: &[&str] = &["zigbee coordinator", "zigbee dongle", "silicon labs"];

pub(super) fn select_best_port(
    ports: &[SerialPortInfo],
    score: impl Fn(&SerialPortInfo) -> Option<u16>,
) -> Option<String> {
    let candidates = ranked_ports(ports, score);
    if candidates.is_empty() {
        return None;
    }

    let best = candidates.first().cloned()?;
    if candidates.len() > 1 {
        let next = &candidates[1];
        if best.0 < 200 && best.0 == next.0 {
            return None;
        }
    }

    Some(best.2)
}

pub(super) fn ranked_port_names(
    ports: &[SerialPortInfo],
    score: impl Fn(&SerialPortInfo) -> Option<u16>,
) -> Vec<String> {
    ranked_ports(ports, score)
        .into_iter()
        .map(|(_, _, name)| name)
        .collect()
}

fn ranked_ports(
    ports: &[SerialPortInfo],
    score: impl Fn(&SerialPortInfo) -> Option<u16>,
) -> Vec<(u16, bool, String)> {
    let mut candidates: Vec<(u16, bool, String)> = ports
        .iter()
        .filter_map(|port| {
            score(port).map(|value| {
                (
                    value,
                    port_name(port).starts_with("/dev/cu."),
                    port.port_name.clone(),
                )
            })
        })
        .collect();
    candidates.sort_by(|left, right| right.cmp(left));
    candidates
}

pub(super) fn base_usb_score(port: &SerialPortInfo) -> Option<u16> {
    if has_vid_pid(port, SONOFF_VID, SONOFF_PID) {
        return Some(320);
    }

    let text = port_text(port);
    matches_any(&text, PRIMARY_USB_IDENTIFIERS).then_some(260)
}

pub(super) fn secondary_usb_score(port: &SerialPortInfo) -> Option<u16> {
    let text = port_text(port);
    matches_any(&text, SECONDARY_USB_IDENTIFIERS).then_some(220)
}

fn has_vid_pid(port: &SerialPortInfo, vid: u16, pid: u16) -> bool {
    let Some(usb) = super::port_description::usb_port(port) else {
        return false;
    };

    usb.vid == vid && usb.pid == pid
}

pub(super) fn port_name(port: &SerialPortInfo) -> String {
    port.port_name.to_ascii_lowercase()
}

fn port_text(port: &SerialPortInfo) -> String {
    let mut fields = vec![port_name(port)];
    if let Some(usb) = super::port_description::usb_port(port) {
        if let Some(manufacturer) = &usb.manufacturer {
            fields.push(manufacturer.to_ascii_lowercase());
        }
        if let Some(product) = &usb.product {
            fields.push(product.to_ascii_lowercase());
        }
        if let Some(serial_number) = &usb.serial_number {
            fields.push(serial_number.to_ascii_lowercase());
        }
    }
    fields.join(" ")
}

fn matches_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}
