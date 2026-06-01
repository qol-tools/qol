use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::open_settings;
#[cfg(target_os = "macos")]
pub use macos::open_settings;

#[cfg(target_os = "linux")]
pub fn detect_coordinator_port(ports: &[SerialPortInfo]) -> Option<String> {
    linux::detect_coordinator_port(ports)
}

#[cfg(target_os = "linux")]
pub fn candidate_coordinator_ports(ports: &[SerialPortInfo]) -> Vec<String> {
    linux::candidate_coordinator_ports(ports)
}

#[cfg(target_os = "macos")]
pub fn detect_coordinator_port(ports: &[SerialPortInfo]) -> Option<String> {
    macos::detect_coordinator_port(ports)
}

#[cfg(target_os = "macos")]
pub fn candidate_coordinator_ports(ports: &[SerialPortInfo]) -> Vec<String> {
    macos::candidate_coordinator_ports(ports)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!(
    "plugin-lights: unsupported target OS; add src/platform/<os>.rs and wire it in src/platform/mod.rs"
);

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

fn select_best_port(
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

fn ranked_port_names(
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

fn base_usb_score(port: &SerialPortInfo) -> Option<u16> {
    if has_vid_pid(port, SONOFF_VID, SONOFF_PID) {
        return Some(320);
    }

    let text = port_text(port);
    if matches_any(&text, PRIMARY_USB_IDENTIFIERS) {
        return Some(260);
    }

    None
}

fn secondary_usb_score(port: &SerialPortInfo) -> Option<u16> {
    let text = port_text(port);
    if matches_any(&text, SECONDARY_USB_IDENTIFIERS) {
        return Some(220);
    }
    None
}

fn has_vid_pid(port: &SerialPortInfo, vid: u16, pid: u16) -> bool {
    let Some(usb) = usb_port(port) else {
        return false;
    };

    usb.vid == vid && usb.pid == pid
}

fn usb_port(port: &SerialPortInfo) -> Option<&UsbPortInfo> {
    let SerialPortType::UsbPort(usb) = &port.port_type else {
        return None;
    };

    Some(usb)
}

fn port_name(port: &SerialPortInfo) -> String {
    port.port_name.to_ascii_lowercase()
}

fn port_text(port: &SerialPortInfo) -> String {
    let mut fields = vec![port_name(port)];
    if let Some(usb) = usb_port(port) {
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

pub fn describe_port(port: &SerialPortInfo) -> String {
    let Some(usb) = usb_port(port) else {
        return port.port_name.clone();
    };

    let mut parts = Vec::new();
    if let Some(manufacturer) = &usb.manufacturer {
        let manufacturer = manufacturer.trim();
        if !manufacturer.is_empty() {
            parts.push(manufacturer.to_string());
        }
    }
    if let Some(product) = &usb.product {
        let product = product.trim();
        if !product.is_empty() {
            parts.push(product.to_string());
        }
    }
    parts.push(format!("{:04x}:{:04x}", usb.vid, usb.pid));

    format!("{} [{}]", port.port_name, parts.join(" · "))
}
