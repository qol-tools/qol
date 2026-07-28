use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use evdev::{Device, KeyCode};

use crate::detection;
use crate::fixes::{DetectedDevice, Mac};
use crate::platform::{
    NativeAdapter, NativeButtonInput, NativeConnection, NativeControllerInput, NativeInputSnapshot,
    NativeSignal, PlatformSupport,
};

const INPUT_DEVICES_PATH: &str = "/proc/bus/input/devices";
const SYSFS_ROOT: &str = "/sys";
const INPUT_DEVICE_ROOT: &str = "/dev/input";
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SIGNAL_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const LEFT_STICK_BUTTON: usize = 10;
const RIGHT_STICK_BUTTON: usize = 11;

pub(crate) fn platform_support() -> PlatformSupport {
    PlatformSupport {
        label: "Linux",
        supported: true,
    }
}

#[derive(Default)]
pub struct InputMonitor {
    devices: Vec<TrackedInput>,
    refreshed_at: Option<Instant>,
    signal_refreshed_at: Option<Instant>,
    signals: HashMap<Mac, Option<NativeSignal>>,
    adapters: HashMap<String, NativeAdapter>,
    device_adapters: HashMap<Mac, String>,
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
            .filter_map(|device| device.snapshot(&self.signals, &self.adapters))
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
        let active_addresses = self
            .devices
            .iter()
            .filter_map(|device| device.bluetooth.as_ref().map(|target| target.address))
            .collect::<HashSet<_>>();
        self.device_adapters
            .retain(|address, _| active_addresses.contains(address));
        for device in &mut self.devices {
            let Some(target) = device.bluetooth.as_mut() else {
                continue;
            };
            let adapter = target
                .adapter
                .clone()
                .or_else(|| self.device_adapters.get(&target.address).cloned())
                .or_else(|| connected_bluez_adapter(target.address, Path::new(SYSFS_ROOT)));
            if let Some(adapter) = adapter {
                self.device_adapters.insert(target.address, adapter.clone());
                target.adapter = Some(adapter);
            }
        }
        let active = self
            .devices
            .iter()
            .filter_map(|device| device.bluetooth.as_ref()?.adapter.clone())
            .collect::<HashSet<_>>();
        self.adapters.retain(|name, _| active.contains(name));
        for name in active {
            self.adapters
                .entry(name.clone())
                .or_insert_with(|| read_bluetooth_adapter(name, Path::new(SYSFS_ROOT)));
        }
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

    fn snapshot(
        &self,
        signals: &HashMap<Mac, Option<NativeSignal>>,
        adapters: &HashMap<String, NativeAdapter>,
    ) -> Option<NativeControllerInput> {
        let keys = self.device.get_key_state().ok()?;
        let signal = self
            .bluetooth
            .as_ref()
            .and_then(|target| signals.get(&target.address).copied().flatten());
        let adapter = self
            .bluetooth
            .as_ref()
            .and_then(|target| target.adapter.as_ref())
            .and_then(|name| adapters.get(name))
            .cloned();
        Some(NativeControllerInput {
            name: self.name.clone(),
            vendor: self.vendor,
            product: self.product,
            connection: NativeConnection {
                transport: self.transport,
                signal,
                adapter,
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
        .find(|part| is_bluetooth_adapter_name(part))
        .map(str::to_string)
}

fn connected_bluez_adapter(address: Mac, sysfs_root: &Path) -> Option<String> {
    let mut adapters = std::fs::read_dir(sysfs_root.join("class/bluetooth"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_bluetooth_adapter_name(name))
        .collect::<Vec<_>>();
    adapters.sort();
    adapters
        .into_iter()
        .find(|adapter| bluez_device_connected(adapter, address))
}

fn is_bluetooth_adapter_name(name: &str) -> bool {
    name.strip_prefix("hci").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
    })
}

fn bluez_device_connected(adapter: &str, address: Mac) -> bool {
    let address = address.to_string().to_ascii_uppercase().replace(':', "_");
    let object_path = format!("/org/bluez/{adapter}/dev_{address}");
    let output = Command::new("busctl")
        .args([
            "--system",
            "--timeout=1s",
            "get-property",
            "org.bluez",
            &object_path,
            "org.bluez.Device1",
            "Connected",
        ])
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| parse_busctl_bool(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or(false)
}

fn bluetooth_rssi(target: &BluetoothTarget) -> Option<NativeSignal> {
    bluez_advertised_rssi(target)
        .map(NativeSignal::AdvertisedDbm)
        .or_else(|| connected_link_rssi(target).map(NativeSignal::BredrLinkMarginDb))
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
    let adapter = target.adapter.as_deref()?;
    let address = target.address.to_string();
    let output = Command::new("hcitool")
        .args(["-i", adapter, "rssi", &address])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_hcitool_rssi(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn read_bluetooth_adapter(name: String, sysfs_root: &Path) -> NativeAdapter {
    let adapter_path = sysfs_root.join("class/bluetooth").join(&name);
    let address =
        read_trimmed(adapter_path.join("address")).or_else(|| bluez_adapter_address(&name));
    let properties = std::fs::canonicalize(adapter_path.join("device"))
        .ok()
        .and_then(|path| read_udev_properties(&path))
        .unwrap_or_default();
    adapter_from_properties(name, address, &properties)
}

fn bluez_adapter_address(adapter: &str) -> Option<String> {
    let object_path = format!("/org/bluez/{adapter}");
    let output = Command::new("busctl")
        .args([
            "--system",
            "--timeout=1s",
            "get-property",
            "org.bluez",
            &object_path,
            "org.bluez.Adapter1",
            "Address",
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_busctl_string(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn read_udev_properties(path: &Path) -> Option<String> {
    let output = Command::new("udevadm")
        .args(["info", "--query=property", "--path"])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn adapter_from_properties(
    name: String,
    address: Option<String>,
    properties: &str,
) -> NativeAdapter {
    NativeAdapter {
        name,
        address,
        vendor: first_property(properties, &["ID_VENDOR_FROM_DATABASE", "ID_VENDOR"]),
        model: first_property(properties, &["ID_MODEL_FROM_DATABASE", "ID_MODEL"]),
        hardware_id: usb_hardware_id(properties),
        path: property(properties, "ID_PATH").map(str::to_string),
    }
}

fn first_property(properties: &str, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| property(properties, key))
        .map(str::to_string)
}

fn property<'a>(properties: &'a str, key: &str) -> Option<&'a str> {
    properties.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key && !value.is_empty()).then_some(value)
    })
}

fn usb_hardware_id(properties: &str) -> Option<String> {
    let explicit = property(properties, "ID_VENDOR_ID")
        .zip(property(properties, "ID_MODEL_ID"))
        .map(|(vendor, product)| format!("{vendor}:{product}"));
    if explicit.is_some() {
        return explicit;
    }
    let mut fields = property(properties, "PRODUCT")?.split('/');
    let vendor = u16::from_str_radix(fields.next()?, 16).ok()?;
    let product = u16::from_str_radix(fields.next()?, 16).ok()?;
    Some(format!("{vendor:04x}:{product:04x}"))
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_busctl_rssi(output: &str) -> Option<i16> {
    let mut fields = output.split_whitespace();
    (fields.next()? == "n")
        .then(|| fields.next()?.parse().ok())
        .flatten()
}

fn parse_busctl_bool(output: &str) -> Option<bool> {
    let mut fields = output.split_whitespace();
    (fields.next()? == "b")
        .then(|| fields.next()?.parse().ok())
        .flatten()
}

fn parse_busctl_string(output: &str) -> Option<String> {
    let mut fields = output.split_whitespace();
    if fields.next()? != "s" {
        return None;
    }
    Some(fields.next()?.trim_matches('"').to_string())
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
    let mut devices = detection::parse_devices(&text);
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
        let mut devices = detection::parse_devices(
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
        let mut devices = detection::parse_devices(
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
        assert_eq!(
            bluetooth_adapter_name("/devices/virtual/misc/uhid/input/input39"),
            None
        );
        for (name, expected) in [
            ("hci0", true),
            ("hci27", true),
            ("hci", false),
            ("hci2:69", false),
            ("foo", false),
        ] {
            assert_eq!(is_bluetooth_adapter_name(name), expected, "{name}");
        }
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
        for (output, expected) in [
            ("b true\n", Some(true)),
            ("b false\n", Some(false)),
            ("s true\n", None),
            ("", None),
        ] {
            assert_eq!(parse_busctl_bool(output), expected, "busctl: {output:?}");
        }
        for (output, expected) in [
            ("s \"00:11:22:33:44:55\"\n", Some("00:11:22:33:44:55")),
            ("s \"\"\n", Some("")),
            ("b true\n", None),
            ("", None),
        ] {
            assert_eq!(
                parse_busctl_string(output).as_deref(),
                expected,
                "busctl: {output:?}"
            );
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

        let adapter = adapter_from_properties(
            "hci7".into(),
            Some("00:11:22:33:44:55".into()),
            "PRODUCT=1234/abcd/1\n\
             ID_VENDOR_FROM_DATABASE=Foo Corp.\n\
             ID_MODEL_FROM_DATABASE=Bar Radio\n\
             ID_PATH=pci-0000:00:01.0-usb-0:2:1.0\n",
        );
        assert_eq!(
            adapter,
            NativeAdapter {
                name: "hci7".into(),
                address: Some("00:11:22:33:44:55".into()),
                vendor: Some("Foo Corp.".into()),
                model: Some("Bar Radio".into()),
                hardware_id: Some("1234:abcd".into()),
                path: Some("pci-0000:00:01.0-usb-0:2:1.0".into()),
            }
        );

        let cases = [
            (
                "ID_VENDOR_ID=0123\nID_MODEL_ID=0045\nPRODUCT=ffff/eeee/1\n",
                Some("0123:0045"),
            ),
            ("PRODUCT=a/2b/1\n", Some("000a:002b")),
            ("PRODUCT=invalid\n", None),
            ("", None),
        ];
        for (properties, expected) in cases {
            assert_eq!(
                usb_hardware_id(properties).as_deref(),
                expected,
                "properties: {properties:?}"
            );
        }
    }
}
