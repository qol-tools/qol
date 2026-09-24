use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use evdev::{AbsInfo, AbsoluteAxisCode, Device, KeyCode};

use crate::detection;
use crate::detection::clash::{self, Holder, LinkEvidence};
use crate::fixes::{DetectedDevice, Mac};
use crate::platform::{
    NativeAdapter, NativeButtonInput, NativeConnection, NativeControllerInput, NativeGamepadAxis,
    NativeGamepadButton, NativeGamepadState, NativeInputSnapshot, NativeSignal, PlatformSupport,
};

const INPUT_DEVICES_PATH: &str = "/proc/bus/input/devices";
const SYSFS_ROOT: &str = "/sys";
const INPUT_DEVICE_ROOT: &str = "/dev/input";
const HID_RAW_DEVICE_ROOT: &str = "/dev";
const KERNEL_LOG_WINDOW: &str = "-60s";
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SIGNAL_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const SIGNAL_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
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
    signals: Option<SignalWorker>,
    adapters: HashMap<String, NativeAdapter>,
    device_adapters: HashMap<Mac, String>,
}

struct SignalWorker {
    state: Arc<Mutex<SignalState>>,
}

#[derive(Default)]
struct SignalState {
    targets: Vec<BluetoothTarget>,
    signals: HashMap<Mac, Option<NativeSignal>>,
    requested_at: Option<Instant>,
}

struct TrackedInput {
    name: String,
    handler: String,
    vendor: u16,
    product: u16,
    transport: &'static str,
    bluetooth: Option<BluetoothTarget>,
    layout: ButtonLayout,
    device: Device,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ButtonLayout {
    Standard,
    GuliKit,
}

impl ButtonLayout {
    fn face_west(self) -> KeyCode {
        match self {
            Self::Standard => KeyCode::BTN_WEST,
            Self::GuliKit => KeyCode::BTN_C,
        }
    }

    fn shoulders(self) -> (KeyCode, KeyCode) {
        match self {
            Self::Standard => (KeyCode::BTN_TL, KeyCode::BTN_TR),
            Self::GuliKit => (KeyCode::BTN_WEST, KeyCode::BTN_Z),
        }
    }

    fn select_start(self) -> (KeyCode, KeyCode) {
        match self {
            Self::Standard => (KeyCode::BTN_SELECT, KeyCode::BTN_START),
            Self::GuliKit => (KeyCode::BTN_TL, KeyCode::BTN_TR),
        }
    }

    fn stick_clicks(self) -> (KeyCode, KeyCode) {
        match self {
            Self::Standard => (KeyCode::BTN_THUMBL, KeyCode::BTN_THUMBR),
            Self::GuliKit => (KeyCode::BTN_TL2, KeyCode::BTN_TR2),
        }
    }
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
        let signals = self.request_signals();
        let items = self
            .devices
            .iter()
            .filter_map(|device| device.snapshot(&signals, &self.adapters))
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

    fn request_signals(&mut self) -> HashMap<Mac, Option<NativeSignal>> {
        let targets = self
            .devices
            .iter()
            .filter_map(|device| device.bluetooth.clone())
            .collect::<Vec<_>>();
        match &self.signals {
            Some(worker) => worker.request(targets),
            None => {
                let signals = collect_signals(&targets);
                self.signals = Some(SignalWorker::start(targets, signals.clone()));
                signals
            }
        }
    }

    fn refresh(&mut self) {
        self.devices = reconcile_devices(std::mem::take(&mut self.devices), &read_devices());
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
}

fn reconcile_devices(open: Vec<TrackedInput>, detected: &[DetectedDevice]) -> Vec<TrackedInput> {
    let mut open = open;
    let mut tracked = Vec::new();
    for device in detected {
        let Some(handler) = device.event_handler.as_deref() else {
            continue;
        };
        let matching = open.iter().position(|input| input.tracks(handler, device));
        match matching {
            Some(index) => tracked.push(open.swap_remove(index)),
            None => tracked.extend(TrackedInput::open(device)),
        }
    }
    tracked
}

fn collect_signals(targets: &[BluetoothTarget]) -> HashMap<Mac, Option<NativeSignal>> {
    targets
        .iter()
        .map(|target| (target.address, bluetooth_rssi(target)))
        .collect()
}

impl SignalWorker {
    fn start(targets: Vec<BluetoothTarget>, signals: HashMap<Mac, Option<NativeSignal>>) -> Self {
        let state = Arc::new(Mutex::new(SignalState {
            targets,
            signals,
            requested_at: Some(Instant::now()),
        }));
        let worker = state.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(SIGNAL_REFRESH_INTERVAL);
            let targets = {
                let state = worker.lock().unwrap_or_else(PoisonError::into_inner);
                match state.requested_at {
                    Some(requested_at) if requested_at.elapsed() < SIGNAL_IDLE_TIMEOUT => {
                        state.targets.clone()
                    }
                    _ => continue,
                }
            };
            let signals = collect_signals(&targets);
            worker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .signals = signals;
        });
        Self { state }
    }

    fn request(&self, targets: Vec<BluetoothTarget>) -> HashMap<Mac, Option<NativeSignal>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.targets = targets;
        state.requested_at = Some(Instant::now());
        state.signals.clone()
    }
}

impl TrackedInput {
    fn tracks(&self, handler: &str, detected: &DetectedDevice) -> bool {
        self.handler == handler
            && self.vendor == detected.vendor
            && self.product == detected.product
            && self.name == detected.name
    }

    fn open(detected: &DetectedDevice) -> Option<Self> {
        if !detected.is_gamepad || detected.is_virtual() {
            return None;
        }
        let handler = detected.event_handler.as_deref()?;
        let device = Device::open(Path::new(INPUT_DEVICE_ROOT).join(handler)).ok()?;
        let layout = button_layout(detected);
        let bluetooth = bluetooth_target(detected);
        Some(Self {
            handler: handler.to_string(),
            name: detected.name.clone(),
            vendor: detected.vendor,
            product: detected.product,
            transport: transport_key(detected.bus),
            bluetooth,
            layout,
            device,
        })
    }

    fn snapshot(
        &self,
        signals: &HashMap<Mac, Option<NativeSignal>>,
        adapters: &HashMap<String, NativeAdapter>,
    ) -> Option<NativeControllerInput> {
        let keys = self.device.get_key_state().ok()?;
        let axes = self
            .device
            .get_absinfo()
            .map(|axes| axes.collect::<HashMap<_, _>>())
            .unwrap_or_default();
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
        let (left_stick, right_stick) = self.layout.stick_clicks();
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
                    pressed: keys.contains(left_stick),
                },
                NativeButtonInput {
                    index: RIGHT_STICK_BUTTON,
                    pressed: keys.contains(right_stick),
                },
            ],
            state: gamepad_state(&keys, &axes, self.layout),
        })
    }
}

fn gamepad_state(
    keys: &evdev::AttributeSet<KeyCode>,
    axes: &HashMap<AbsoluteAxisCode, AbsInfo>,
    layout: ButtonLayout,
) -> NativeGamepadState {
    let (left_stick, right_stick) = layout.stick_clicks();
    let (left_shoulder, right_shoulder) = layout.shoulders();
    let (select, start) = layout.select_start();
    let left_trigger = unipolar_axis(axes.get(&AbsoluteAxisCode::ABS_Z));
    let right_trigger = unipolar_axis(axes.get(&AbsoluteAxisCode::ABS_RZ));
    let hat_x = bipolar_axis(axes.get(&AbsoluteAxisCode::ABS_HAT0X));
    let hat_y = bipolar_axis(axes.get(&AbsoluteAxisCode::ABS_HAT0Y));
    let specs = [
        (0, "South", key_value(keys, KeyCode::BTN_SOUTH)),
        (1, "East", key_value(keys, KeyCode::BTN_EAST)),
        (2, "West", key_value(keys, layout.face_west())),
        (3, "North", key_value(keys, KeyCode::BTN_NORTH)),
        (4, "Left shoulder", key_value(keys, left_shoulder)),
        (5, "Right shoulder", key_value(keys, right_shoulder)),
        (
            6,
            "Left trigger",
            left_trigger.max(digital_trigger_value(keys, KeyCode::BTN_TL2, left_stick)),
        ),
        (
            7,
            "Right trigger",
            right_trigger.max(digital_trigger_value(keys, KeyCode::BTN_TR2, right_stick)),
        ),
        (8, "Select", key_value(keys, select)),
        (9, "Start", key_value(keys, start)),
        (10, "Left stick", key_value(keys, left_stick)),
        (11, "Right stick", key_value(keys, right_stick)),
        (
            12,
            "D-pad up",
            key_value(keys, KeyCode::BTN_DPAD_UP).max((-hat_y).max(0.0)),
        ),
        (
            13,
            "D-pad down",
            key_value(keys, KeyCode::BTN_DPAD_DOWN).max(hat_y.max(0.0)),
        ),
        (
            14,
            "D-pad left",
            key_value(keys, KeyCode::BTN_DPAD_LEFT).max((-hat_x).max(0.0)),
        ),
        (
            15,
            "D-pad right",
            key_value(keys, KeyCode::BTN_DPAD_RIGHT).max(hat_x.max(0.0)),
        ),
        (16, "Home", key_value(keys, KeyCode::BTN_MODE)),
    ];
    let buttons = specs
        .into_iter()
        .map(|(index, name, value)| NativeGamepadButton {
            index,
            name,
            pressed: value > 0.05,
            value,
        })
        .collect();
    let axes = [
        (0, "Left X", AbsoluteAxisCode::ABS_X),
        (1, "Left Y", AbsoluteAxisCode::ABS_Y),
        (2, "Right X", AbsoluteAxisCode::ABS_RX),
        (3, "Right Y", AbsoluteAxisCode::ABS_RY),
    ]
    .into_iter()
    .map(|(index, name, code)| NativeGamepadAxis {
        index,
        name,
        value: bipolar_axis(axes.get(&code)),
    })
    .collect();
    NativeGamepadState {
        mapping: "standard",
        buttons,
        axes,
    }
}

fn key_value(keys: &evdev::AttributeSet<KeyCode>, key: KeyCode) -> f32 {
    if keys.contains(key) {
        1.0
    } else {
        0.0
    }
}

fn digital_trigger_value(
    keys: &evdev::AttributeSet<KeyCode>,
    trigger: KeyCode,
    remapped_stick: KeyCode,
) -> f32 {
    if trigger == remapped_stick {
        return 0.0;
    }
    key_value(keys, trigger)
}

fn bipolar_axis(info: Option<&AbsInfo>) -> f32 {
    normalize_axis(info, -1.0, 1.0)
}

fn unipolar_axis(info: Option<&AbsInfo>) -> f32 {
    normalize_axis(info, 0.0, 1.0)
}

fn normalize_axis(info: Option<&AbsInfo>, low: f32, high: f32) -> f32 {
    let Some(info) = info else {
        return 0.0;
    };
    let span = info.maximum() - info.minimum();
    if span <= 0 {
        return 0.0;
    }
    let unit = (info.value() - info.minimum()) as f32 / span as f32;
    (low + unit * (high - low)).clamp(low, high)
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

fn bluez_device_path(adapter: &str, address: Mac) -> String {
    let address = address.to_string().to_ascii_uppercase().replace(':', "_");
    format!("/org/bluez/{adapter}/dev_{address}")
}

fn bluez_device_connected(adapter: &str, address: Mac) -> bool {
    let object_path = bluez_device_path(adapter, address);
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
    let object_path = bluez_device_path(adapter, target.address);
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

fn button_layout(device: &DetectedDevice) -> ButtonLayout {
    if device.vendor == 0x045e
        && device.product == 0x02e0
        && device.name == "GuliKit Controller XW"
        && device
            .driver
            .as_deref()
            .is_some_and(|driver| matches!(driver, "hid-generic" | "hid_generic"))
    {
        return ButtonLayout::GuliKit;
    }
    ButtonLayout::Standard
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

pub fn link_evidence(devices: &[DetectedDevice]) -> Vec<Option<LinkEvidence>> {
    let candidates = devices.iter().map(link_candidate).collect::<Vec<_>>();
    if candidates.iter().all(Option::is_none) {
        return vec![None; devices.len()];
    }
    let log = kernel_log();
    devices
        .iter()
        .zip(candidates)
        .map(|(device, candidate)| {
            let hid_id = candidate?;
            Some(LinkEvidence {
                driver_timeouts: clash::count_input_timeouts(&log, hid_id),
                sticks_pinned: stick_pinning(device),
                holders: hidraw_holders(device),
            })
        })
        .collect()
}

fn link_candidate(device: &DetectedDevice) -> Option<&str> {
    if !device.is_gamepad || device.is_virtual() {
        return None;
    }
    clash::hid_id(device.sysfs_path.as_deref()?)
}

fn stick_pinning(device: &DetectedDevice) -> Option<bool> {
    let handler = device.event_handler.as_deref()?;
    let input = Device::open(Path::new(INPUT_DEVICE_ROOT).join(handler)).ok()?;
    let axes = input.get_absinfo().ok()?.collect::<HashMap<_, _>>();
    let readings = [
        AbsoluteAxisCode::ABS_X,
        AbsoluteAxisCode::ABS_Y,
        AbsoluteAxisCode::ABS_RX,
        AbsoluteAxisCode::ABS_RY,
    ]
    .into_iter()
    .filter_map(|code| {
        let info = axes.get(&code)?;
        Some(clash::AxisReading {
            value: info.value(),
            minimum: info.minimum(),
            maximum: info.maximum(),
        })
    })
    .collect::<Vec<_>>();
    clash::sticks_pinned(&readings)
}

fn kernel_log() -> String {
    Command::new("journalctl")
        .arg("-k")
        .arg("-b")
        .arg(format!("--since={KERNEL_LOG_WINDOW}"))
        .arg("-o")
        .arg("cat")
        .arg("--no-pager")
        .arg("-q")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

pub fn hidraw_holders(device: &DetectedDevice) -> Vec<Holder> {
    let Some(sysfs_path) = device.sysfs_path.as_deref() else {
        return Vec::new();
    };
    let Some(hid_id) = clash::hid_id(sysfs_path) else {
        return Vec::new();
    };
    hidraw_holders_in(
        Path::new("/proc"),
        Path::new(SYSFS_ROOT),
        sysfs_path,
        hid_id,
    )
}

fn hidraw_holders_in(
    proc_root: &Path,
    sysfs_root: &Path,
    sysfs_path: &str,
    hid_id: &str,
) -> Vec<Holder> {
    let nodes = hidraw_nodes(sysfs_root, sysfs_path, hid_id);
    if nodes.is_empty() {
        return Vec::new();
    }
    let targets = nodes
        .iter()
        .map(|node| format!("{HID_RAW_DEVICE_ROOT}/{node}"))
        .collect::<HashSet<_>>();
    let mut holders = HashMap::new();
    let self_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        if !process_holds_raw_device(&entry.path(), &targets) {
            continue;
        }
        let name = read_trimmed(entry.path().join("comm")).unwrap_or_default();
        holders.insert(pid, Holder { pid, name });
    }
    let mut holders = holders.into_values().collect::<Vec<_>>();
    holders.sort_by_key(|holder| holder.pid);
    holders
}

fn process_holds_raw_device(process_root: &Path, targets: &HashSet<String>) -> bool {
    let Ok(entries) = std::fs::read_dir(process_root.join("fd")) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        std::fs::read_link(entry.path())
            .ok()
            .is_some_and(|link| targets.contains(link.to_string_lossy().as_ref()))
    })
}

fn hidraw_nodes(sysfs_root: &Path, sysfs_path: &str, hid_id: &str) -> Vec<String> {
    let Some(hid_dir) = hid_directory(sysfs_path, hid_id) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(sysfs_root.join(hid_dir).join("hidraw")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

fn hid_directory<'a>(sysfs_path: &'a str, hid_id: &str) -> Option<&'a str> {
    let start = sysfs_path.find(hid_id)?;
    let end = start + hid_id.len();
    let rest = sysfs_path.get(end..)?;
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    sysfs_path.get(..end)?.strip_prefix('/')
}

pub fn disconnect_bluetooth(device: &DetectedDevice) -> Result<String> {
    if device.bus != 0x0005 {
        bail!("{} is not a Bluetooth controller", device.name);
    }
    let Some(target) = bluetooth_target(device) else {
        bail!("{} has no usable Bluetooth address", device.name);
    };
    let adapter = target
        .adapter
        .or_else(|| connected_bluez_adapter(target.address, Path::new(SYSFS_ROOT)))
        .with_context(|| {
            format!(
                "could not prove which Bluetooth adapter holds {}",
                device.name
            )
        })?;
    let object_path = bluez_device_path(&adapter, target.address);
    let output = Command::new("busctl")
        .args([
            "--system",
            "--timeout=5s",
            "call",
            "org.bluez",
            &object_path,
            "org.bluez.Device1",
            "Disconnect",
        ])
        .output()
        .context("failed to run busctl")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(adapter)
}

pub fn stop_process(pid: u32) -> Result<()> {
    let output = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .output()
        .context("failed to run kill")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn holder_process(proc_root: &Path, pid: &str, node: &str, comm: &str) {
        let fd = proc_root.join(pid).join("fd");
        std::fs::create_dir_all(&fd).expect("fd dir");
        symlink(format!("/dev/{node}"), fd.join("3")).expect("fd symlink");
        std::fs::write(proc_root.join(pid).join("comm"), format!("{comm}\n")).expect("comm");
    }

    #[test]
    fn bluez_device_path_formats_adapter_and_mac() {
        assert_eq!(
            bluez_device_path("hci1", Mac::parse("00:11:22:33:44:55").expect("mac")),
            "/org/bluez/hci1/dev_00_11_22_33_44_55"
        );
    }

    #[test]
    fn hidraw_holders_reports_only_processes_holding_the_hid_device() {
        let root = tempfile::tempdir().expect("tempdir");
        let sysfs = root.path().join("sys");
        let proc = root.path().join("proc");
        let sysfs_path =
            "/devices/pci0000:00/bluetooth/hci0/hci0:256/0005:057E:2009.000F/input/input52";
        let hidraw =
            sysfs.join("devices/pci0000:00/bluetooth/hci0/hci0:256/0005:057E:2009.000F/hidraw");
        std::fs::create_dir_all(&hidraw).expect("hidraw dir");
        std::fs::write(hidraw.join("hidraw13"), "").expect("hidraw node");

        holder_process(&proc, "4242", "hidraw13", "steam");
        holder_process(&proc, "77", "hidraw2", "other");
        std::fs::create_dir_all(proc.join("88")).expect("missing fd dir");
        std::fs::write(proc.join("88/comm"), "lonely\n").expect("comm");
        holder_process(&proc, "notapid", "hidraw13", "ignored");

        assert_eq!(
            hidraw_holders_in(&proc, &sysfs, sysfs_path, "0005:057E:2009.000F"),
            vec![Holder {
                pid: 4242,
                name: "steam".into(),
            }]
        );
    }

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
    fn button_layout_applies_only_the_gulikit_hid_generic_quirk() {
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

        let gulikit = button_layout(&devices.remove(0));
        let xpadneo = button_layout(&devices.remove(0));
        let standard = button_layout(&devices.remove(0));

        assert_eq!(gulikit, ButtonLayout::GuliKit);
        assert_eq!(xpadneo, ButtonLayout::Standard);
        assert_eq!(standard, ButtonLayout::Standard);
    }

    #[test]
    fn gulikit_layout_remaps_shifted_hid_generic_button_codes() {
        let cases = [
            (KeyCode::BTN_SOUTH, 0),
            (KeyCode::BTN_EAST, 1),
            (KeyCode::BTN_C, 2),
            (KeyCode::BTN_NORTH, 3),
            (KeyCode::BTN_WEST, 4),
            (KeyCode::BTN_Z, 5),
            (KeyCode::BTN_TL, 8),
            (KeyCode::BTN_TR, 9),
            (KeyCode::BTN_TL2, 10),
            (KeyCode::BTN_TR2, 11),
        ];
        for (code, expected_index) in cases {
            let keys = evdev::AttributeSet::from_iter([code]);
            let state = gamepad_state(&keys, &HashMap::new(), ButtonLayout::GuliKit);
            let pressed = state
                .buttons
                .iter()
                .filter(|button| button.pressed)
                .map(|button| button.index)
                .collect::<Vec<_>>();
            assert_eq!(pressed, [expected_index], "code: {code:?}");
        }
    }

    #[test]
    fn axis_normalization_maps_kernel_ranges_to_gamepad_ranges() {
        let cases = [
            (AbsInfo::new(0, 0, 255, 0, 0, 0), 0.0, 1.0, 0.0),
            (AbsInfo::new(255, 0, 255, 0, 0, 0), 0.0, 1.0, 1.0),
            (AbsInfo::new(128, 0, 256, 0, 0, 0), -1.0, 1.0, 0.0),
            (
                AbsInfo::new(-32768, -32768, 32767, 0, 0, 0),
                -1.0,
                1.0,
                -1.0,
            ),
            (AbsInfo::new(32767, -32768, 32767, 0, 0, 0), -1.0, 1.0, 1.0),
        ];
        for (info, low, high, expected) in cases {
            let actual = normalize_axis(Some(&info), low, high);
            assert!((actual - expected).abs() < 0.0001, "info: {info:?}");
        }
        assert_eq!(normalize_axis(None, -1.0, 1.0), 0.0);
        assert_eq!(
            normalize_axis(Some(&AbsInfo::new(3, 3, 3, 0, 0, 0)), -1.0, 1.0),
            0.0
        );
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
