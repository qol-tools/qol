use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use evdev::{Device, KeyCode};

use crate::detect;
use crate::fixes::{DetectedDevice, Mac};
use crate::platform::{
    NativeButtonInput, NativeConnection, NativeControllerInput, NativeInputSnapshot,
};

const INPUT_DEVICES_PATH: &str = "/proc/bus/input/devices";
const SYSFS_ROOT: &str = "/sys";
const INPUT_DEVICE_ROOT: &str = "/dev/input";
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SIGNAL_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const LEFT_STICK_BUTTON: usize = 10;
const RIGHT_STICK_BUTTON: usize = 11;

#[derive(Default)]
pub struct InputMonitor {
    devices: Vec<TrackedInput>,
    refreshed_at: Option<Instant>,
    signal_refreshed_at: Option<Instant>,
    signals: HashMap<Mac, Option<i16>>,
}

struct TrackedInput {
    name: String,
    vendor: u16,
    product: u16,
    transport: &'static str,
    bluetooth: Option<BluetoothTarget>,
    left_stick: KeyCode,
    right_stick: KeyCode,
    device: Device,
}

#[derive(Clone)]
struct BluetoothTarget {
    adapter: Option<String>,
    address: Mac,
}

impl InputMonitor {
    pub fn snapshot(&mut self) -> NativeInputSnapshot {
        if self.should_refresh() {
            self.refresh();
        }
        if self.should_refresh_signal() {
            self.refresh_signals();
        }
        let items = self
            .devices
            .iter()
            .filter_map(|device| device.snapshot(&self.signals))
            .collect();
        NativeInputSnapshot {
            available: true,
            source: Some("linux-evdev"),
            items,
        }
    }

    fn should_refresh(&self) -> bool {
        self.refreshed_at
            .is_none_or(|time| time.elapsed() >= DEVICE_REFRESH_INTERVAL)
    }

    fn refresh(&mut self) {
        self.devices = read_devices()
            .iter()
            .filter_map(TrackedInput::open)
            .collect();
        self.refreshed_at = Some(Instant::now());
    }

    fn should_refresh_signal(&self) -> bool {
        self.signal_refreshed_at
            .is_none_or(|time| time.elapsed() >= SIGNAL_REFRESH_INTERVAL)
    }

    fn refresh_signals(&mut self) {
        let active = self
            .devices
            .iter()
            .filter_map(|device| device.bluetooth.as_ref().map(|target| target.address))
            .collect::<HashSet<_>>();
        self.signals.retain(|address, _| active.contains(address));
        for target in self
            .devices
            .iter()
            .filter_map(|device| device.bluetooth.as_ref())
        {
            self.signals.insert(target.address, bluetooth_rssi(target));
        }
        self.signal_refreshed_at = Some(Instant::now());
    }
}

impl TrackedInput {
    fn open(detected: &DetectedDevice) -> Option<Self> {
        if !detected.is_gamepad || detected.is_virtual() {
            return None;
        }
        let handler = detected.event_handler.as_deref()?;
        let device = Device::open(Path::new(INPUT_DEVICE_ROOT).join(handler)).ok()?;
        let (left_stick, right_stick) = stick_click_codes(detected);
        let bluetooth = bluetooth_target(detected);
        Some(Self {
            name: detected.name.clone(),
            vendor: detected.vendor,
            product: detected.product,
            transport: transport_key(detected.bus),
            bluetooth,
            left_stick,
            right_stick,
            device,
        })
    }

    fn snapshot(&self, signals: &HashMap<Mac, Option<i16>>) -> Option<NativeControllerInput> {
        let keys = self.device.get_key_state().ok()?;
        let signal_dbm = self
            .bluetooth
            .as_ref()
            .and_then(|target| signals.get(&target.address).copied().flatten());
        Some(NativeControllerInput {
            name: self.name.clone(),
            vendor: self.vendor,
            product: self.product,
            connection: NativeConnection {
                transport: self.transport,
                signal_dbm,
            },
            buttons: vec![
                NativeButtonInput {
                    index: LEFT_STICK_BUTTON,
                    pressed: keys.contains(self.left_stick),
                },
                NativeButtonInput {
                    index: RIGHT_STICK_BUTTON,
                    pressed: keys.contains(self.right_stick),
                },
            ],
        })
    }
}

fn transport_key(bus: u16) -> &'static str {
    match bus {
        0x0005 => "bluetooth",
        0x0003 => "usb",
        _ => "other",
    }
}

fn bluetooth_target(device: &DetectedDevice) -> Option<BluetoothTarget> {
    if device.bus != 0x0005 {
        return None;
    }
    let address = device.uniq.as_deref().and_then(Mac::parse)?;
    let adapter = device
        .sysfs_path
        .as_deref()
        .and_then(bluetooth_adapter_name);
    Some(BluetoothTarget { adapter, address })
}

fn bluetooth_adapter_name(sysfs_path: &str) -> Option<String> {
    sysfs_path
        .split('/')
        .find(|part| {
            part.strip_prefix("hci").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
            })
        })
        .map(str::to_string)
}

fn bluetooth_rssi(target: &BluetoothTarget) -> Option<i16> {
    bluez_advertised_rssi(target).or_else(|| connected_link_rssi(target))
}

fn bluez_advertised_rssi(target: &BluetoothTarget) -> Option<i16> {
    let adapter = target.adapter.as_deref()?;
    let address = target
        .address
        .to_string()
        .to_ascii_uppercase()
        .replace(':', "_");
    let object_path = format!("/org/bluez/{adapter}/dev_{address}");
    let output = Command::new("busctl")
        .args([
            "--system",
            "--timeout=1s",
            "get-property",
            "org.bluez",
            &object_path,
            "org.bluez.Device1",
            "RSSI",
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_busctl_rssi(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn connected_link_rssi(target: &BluetoothTarget) -> Option<i16> {
    let address = target.address.to_string();
    let mut command = Command::new("hcitool");
    if let Some(adapter) = target.adapter.as_deref() {
        command.args(["-i", adapter]);
    }
    let output = command.args(["rssi", &address]).output().ok()?;
    output
        .status
        .success()
        .then(|| parse_hcitool_rssi(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn parse_busctl_rssi(output: &str) -> Option<i16> {
    let mut fields = output.split_whitespace();
    (fields.next()? == "n")
        .then(|| fields.next()?.parse().ok())
        .flatten()
}

fn parse_hcitool_rssi(output: &str) -> Option<i16> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("RSSI return value:"))?
        .trim()
        .parse()
        .ok()
}

fn stick_click_codes(device: &DetectedDevice) -> (KeyCode, KeyCode) {
    if device.vendor == 0x045e
        && device.product == 0x02e0
        && device.name == "GuliKit Controller XW"
        && device
            .driver
            .as_deref()
            .is_some_and(|driver| matches!(driver, "hid-generic" | "hid_generic"))
    {
        return (KeyCode::BTN_TL2, KeyCode::BTN_TR2);
    }
    (KeyCode::BTN_THUMBL, KeyCode::BTN_THUMBR)
}

pub fn read_devices() -> Vec<DetectedDevice> {
    let Ok(text) = std::fs::read_to_string(INPUT_DEVICES_PATH) else {
        return Vec::new();
    };
    let mut devices = detect::parse_devices(&text);
    populate_drivers(&mut devices, Path::new(SYSFS_ROOT));
    devices
}

fn populate_drivers(devices: &mut [DetectedDevice], sysfs_root: &Path) {
    for device in devices {
        let Some(path) = device.sysfs_path.as_deref() else {
            continue;
        };
        device.driver = driver_name(sysfs_root, path);
    }
}

fn driver_name(sysfs_root: &Path, sysfs_path: &str) -> Option<String> {
    let relative = sysfs_path.strip_prefix('/')?;
    let link = std::fs::read_link(sysfs_root.join(relative).join("device/driver")).ok()?;
    link.file_name()?.to_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn driver_name_reads_the_bound_sysfs_driver() {
        let root = tempfile::tempdir().expect("tempdir");
        let input = root.path().join("devices/hid/input/input39/device");
        std::fs::create_dir_all(&input).expect("input path");
        symlink("/sys/bus/hid/drivers/hid-generic", input.join("driver")).expect("driver symlink");

        assert_eq!(
            driver_name(root.path(), "/devices/hid/input/input39").as_deref(),
            Some("hid-generic")
        );
    }

    #[test]
    fn populate_drivers_preserves_devices_without_a_driver_link() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut devices = detect::parse_devices(
            "I: Bus=0003 Vendor=28de Product=11ff Version=0001\n\
             N: Name=\"Virtual pad\"\n\
             S: Sysfs=/devices/virtual/input/input40\n\
             H: Handlers=event22 js1\n",
        );

        populate_drivers(&mut devices, root.path());

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].driver, None);
        assert_eq!(devices[0].driver_label(), "userspace");
    }

    #[test]
    fn stick_click_codes_apply_only_the_gulikit_bluetooth_quirk() {
        let mut devices = detect::parse_devices(
            "I: Bus=0005 Vendor=045e Product=02e0 Version=0903\n\
             N: Name=\"GuliKit Controller XW\"\n\
             H: Handlers=event21 js0\n\n\
             I: Bus=0005 Vendor=045e Product=02e0 Version=0903\n\
             N: Name=\"GuliKit Controller XW\"\n\
             H: Handlers=event23 js2\n\n\
             I: Bus=0003 Vendor=045e Product=028e Version=0114\n\
             N: Name=\"Xbox Controller\"\n\
             H: Handlers=event22 js1\n",
        );
        devices[0].driver = Some("hid-generic".into());
        devices[1].driver = Some("xpadneo".into());

        let gulikit = stick_click_codes(&devices.remove(0));
        let xpadneo = stick_click_codes(&devices.remove(0));
        let standard = stick_click_codes(&devices.remove(0));

        assert_eq!(gulikit, (KeyCode::BTN_TL2, KeyCode::BTN_TR2));
        assert_eq!(xpadneo, (KeyCode::BTN_THUMBL, KeyCode::BTN_THUMBR));
        assert_eq!(standard, (KeyCode::BTN_THUMBL, KeyCode::BTN_THUMBR));
    }

    #[test]
    fn bluetooth_metadata_parses_adapter_and_signal_outputs() {
        assert_eq!(
            bluetooth_adapter_name("/devices/pci0000:00/bluetooth/hci2/input/input39").as_deref(),
            Some("hci2")
        );
        assert_eq!(bluetooth_adapter_name("/devices/usb/input/input4"), None);

        let cases = [
            ("n -24\n", Some(-24)),
            ("n -127\n", Some(-127)),
            ("v -24\n", None),
            ("", None),
        ];
        for (output, expected) in cases {
            assert_eq!(parse_busctl_rssi(output), expected, "busctl: {output:?}");
        }

        let cases = [
            ("RSSI return value: -24\n", Some(-24)),
            ("RSSI return value: 7\n", Some(7)),
            (
                "Get connection info failed: No such file or directory\n",
                None,
            ),
        ];
        for (output, expected) in cases {
            assert_eq!(parse_hcitool_rssi(output), expected, "hcitool: {output:?}");
        }
    }
}
