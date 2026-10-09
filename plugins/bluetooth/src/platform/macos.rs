use std::ffi::CString;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use objc2::rc::Retained;
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::{NSArray, NSString};
use objc2_io_bluetooth::{
    BluetoothHCIPowerState, IOBluetoothDevice, IOBluetoothDeviceInquiry, IOBluetoothDevicePair,
    IOBluetoothHostController, IOBluetoothSDPUUID,
};
use qol_headless::DoctorCheckResult;

use crate::bluetooth::{
    normalize_address, AdapterHealth, BackendCapabilities, DeviceInfo, ReconnectReport,
    ReconnectSelection,
};
use crate::config::ReconnectConfig;

use super::paired::{self, PairedStack};
pub use super::paired::{search_status_snapshot, settings_action, settings_query, stop_search};

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
    audio_reclaim: crate::audio_claim::platform::RECLAIM_SUPPORTED,
};

const AUDIO_SINK_UUID16: u16 = 0x110b;
const AUDIO_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const IO_RETURN_SUCCESS: i32 = 0;
const RSSI_UNAVAILABLE: i8 = 127;
const SEARCH_SECONDS: u8 = 10;
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_LOOP_SLICE: Duration = Duration::from_millis(100);
const ADAPTER_POWER_SYMBOL: &str = "IOBluetoothPreferenceSetControllerPowerState";
const TRUST_UNSUPPORTED: &str =
    "macOS pairs and trusts in one step, so there is no separate trust to change";

struct MacOs;

impl PairedStack for MacOs {
    const TRUST_REFUSAL: &'static str = TRUST_UNSUPPORTED;
    const RETRY_SPACING: Duration = Duration::ZERO;

    fn paired_devices() -> Vec<DeviceInfo> {
        paired_devices()
    }

    fn adapter_health() -> Result<AdapterHealth> {
        adapter_health()
    }

    fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
        set_adapter_powered(powered)
    }

    fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
        connect_device(address, power_on_adapter)
    }

    fn disconnect_device(address: &str) -> Result<DeviceInfo> {
        disconnect_device(address)
    }

    fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
        pair_device(address, power_on_adapter)
    }

    fn remove_device(address: &str) -> Result<()> {
        remove_device(address)
    }

    fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
        search_devices(config)
    }

    fn pause(slice: Duration) {
        deliver_pending_iobluetooth_callbacks(slice);
    }
}

fn deliver_pending_iobluetooth_callbacks(slice: Duration) {
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, slice.as_secs_f64(), false);
}

fn deliver_callbacks_until(timeout: Duration, mut satisfied: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if satisfied() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        deliver_pending_iobluetooth_callbacks(RUN_LOOP_SLICE);
    }
}

fn set_controller_power_state(powered: bool) -> Result<()> {
    type SetPowerState = unsafe extern "C" fn(i32) -> i32;

    let name = CString::new(ADAPTER_POWER_SYMBOL).expect("symbol name has no interior nul");
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if symbol.is_null() {
        bail!("this macOS build exposes no adapter power control for Bluetooth");
    }
    let set_power_state =
        unsafe { std::mem::transmute::<*mut libc::c_void, SetPowerState>(symbol) };
    let result = unsafe { set_power_state(i32::from(powered)) };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS refused to change the Bluetooth adapter power state (code {result})");
    }
    Ok(())
}

fn remove_pairing(device: &IOBluetoothDevice) -> Result<()> {
    let selector = objc2::sel!(remove);
    let exposes_unpair: bool = unsafe { objc2::msg_send![device, respondsToSelector: selector] };
    if !exposes_unpair {
        bail!("this macOS build exposes no unpair operation for Bluetooth devices");
    }
    let result: i32 = unsafe { objc2::msg_send![device, remove] };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS refused to unpair the device (code {result})");
    }
    Ok(())
}

fn host_controller() -> Result<Retained<IOBluetoothHostController>> {
    unsafe { IOBluetoothHostController::defaultController() }
        .ok_or_else(|| anyhow!("this Mac has no Bluetooth controller"))
}

fn controller_is_powered(controller: &IOBluetoothHostController) -> bool {
    let state: i32 = unsafe { objc2::msg_send![controller, powerState] };
    state == BluetoothHCIPowerState::ON.0 as i32
}

fn colon_separated_address(iobluetooth_address: &str) -> String {
    iobluetooth_address.replace('-', ":").to_ascii_uppercase()
}

fn device_address(device: &IOBluetoothDevice) -> Result<String> {
    let raw = unsafe { device.addressString() }
        .ok_or_else(|| anyhow!("a Bluetooth device reported no address"))?;
    normalize_address(&colon_separated_address(&raw.to_string()))
}

fn device_alias(device: &IOBluetoothDevice, address: &str) -> String {
    unsafe { device.nameOrAddress() }
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.to_string())
}

fn cached_audio_sink_service(device: &IOBluetoothDevice) -> (bool, Vec<String>) {
    let Some(uuid) = (unsafe { IOBluetoothSDPUUID::uuid16(AUDIO_SINK_UUID16) }) else {
        return (false, Vec::new());
    };
    let services_resolved = unsafe { device.services() }.is_some();
    let advertises_sink = unsafe { device.getServiceRecordForUUID(Some(&uuid)) }.is_some();
    let uuids = if advertises_sink {
        vec![AUDIO_SINK_UUID.to_string()]
    } else {
        Vec::new()
    };
    (services_resolved, uuids)
}

fn device_info(device: &IOBluetoothDevice) -> Result<DeviceInfo> {
    let address = device_address(device)?;
    let alias = device_alias(device, &address);
    let paired = unsafe { device.isPaired() };
    let (services_resolved, uuids) = cached_audio_sink_service(device);
    let rssi = unsafe { device.RSSI() };
    Ok(DeviceInfo {
        alias,
        trusted: paired,
        paired,
        connected: unsafe { device.isConnected() },
        audio_connected: None,
        services_resolved,
        icon: None,
        class: Some(unsafe { device.classOfDevice() }),
        uuids,
        rssi: (rssi != RSSI_UNAVAILABLE).then_some(i16::from(rssi)),
        address,
    })
}

fn richer_duplicate(existing: &DeviceInfo, candidate: &DeviceInfo) -> bool {
    (candidate.connected, candidate.paired, candidate.uuids.len())
        > (existing.connected, existing.paired, existing.uuids.len())
}

fn readable_device_infos(devices: Option<Retained<NSArray>>) -> Vec<DeviceInfo> {
    let Some(devices) = devices else {
        return Vec::new();
    };
    let mut ordered: Vec<DeviceInfo> = Vec::new();
    for object in devices.iter() {
        let device = unsafe { Retained::cast_unchecked::<IOBluetoothDevice>(object) };
        let Ok(info) = device_info(&device) else {
            continue;
        };
        match ordered
            .iter_mut()
            .find(|existing| existing.address == info.address)
        {
            Some(existing) => {
                if richer_duplicate(existing, &info) {
                    *existing = info;
                }
            }
            None => ordered.push(info),
        }
    }
    ordered
}

fn paired_devices() -> Vec<DeviceInfo> {
    readable_device_infos(unsafe { IOBluetoothDevice::pairedDevices() })
}

fn device_handle(address: &str) -> Result<Retained<IOBluetoothDevice>> {
    let address = normalize_address(address)?;
    let text = NSString::from_str(&address);
    unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&text)) }
        .ok_or_else(|| anyhow!("macOS knows no Bluetooth device at {address}"))
}

pub fn required_binaries_check() -> DoctorCheckResult {
    fn details(controller: bool, powered: Option<bool>) -> serde_json::Value {
        serde_json::json!({
            "platform": "macos",
            "controller": controller,
            "powered": powered,
            "executed": false,
        })
    }

    let Some(controller) = (unsafe { IOBluetoothHostController::defaultController() }) else {
        return DoctorCheckResult::fail(
            "required_binaries",
            "macOS reported no default Bluetooth controller",
        )
        .with_fix("Check that this Mac has Bluetooth hardware and that it is not disabled")
        .with_details(details(false, None));
    };
    let powered = controller_is_powered(&controller);
    DoctorCheckResult::ok(
        "required_binaries",
        "The macOS IOBluetooth controller is available",
    )
    .with_details(details(true, Some(powered)))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    Ok(paired_devices())
}

pub fn adapter_health() -> Result<AdapterHealth> {
    let controller = host_controller()?;
    let name = unsafe { controller.nameAsString() }
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Bluetooth".to_string());
    let address = unsafe { controller.addressAsString() }
        .map(|address| colon_separated_address(&address.to_string()))
        .unwrap_or_default();
    Ok(AdapterHealth {
        name,
        address,
        powered: controller_is_powered(&controller),
    })
}

pub fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
    let result = set_controller_power_state(powered);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_ADAPTER_POWER",
        "source=cli powered={powered} outcome={outcome}"
    );
    result?;
    deliver_callbacks_until(CONNECT_TIMEOUT, || {
        adapter_health().is_ok_and(|health| health.powered == powered)
    });
    adapter_health()
}

pub fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    paired::ensure_powered::<MacOs>(power_on_adapter)?;
    let device = device_handle(address)?;
    if unsafe { device.isConnected() } {
        return device_info(&device);
    }
    let result = unsafe { device.openConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not connect to {address} (code {result})");
    }
    if !deliver_callbacks_until(CONNECT_TIMEOUT, || unsafe { device.isConnected() }) {
        bail!("{address} did not finish connecting");
    }
    device_info(&device)
}

pub fn disconnect_device(address: &str) -> Result<DeviceInfo> {
    let device = device_handle(address)?;
    let result = unsafe { device.closeConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not disconnect {address} (code {result})");
    }
    device_info(&device)
}

pub fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    paired::ensure_powered::<MacOs>(power_on_adapter)?;
    let device = device_handle(address)?;
    if unsafe { device.isPaired() } {
        return device_info(&device);
    }
    let pairing = unsafe { IOBluetoothDevicePair::pairWithDevice(Some(&device)) }
        .ok_or_else(|| anyhow!("macOS could not start pairing with {address}"))?;
    let started = unsafe { pairing.start() };
    if started != IO_RETURN_SUCCESS {
        bail!("macOS refused to pair with {address} (code {started})");
    }
    let paired = deliver_callbacks_until(PAIR_TIMEOUT, || unsafe { device.isPaired() });
    unsafe { pairing.stop() };
    if !paired {
        bail!("pairing with {address} did not complete");
    }
    device_info(&device)
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!(TRUST_UNSUPPORTED)
}

pub fn remove_device(address: &str) -> Result<()> {
    let device = device_handle(address)?;
    remove_pairing(&device)
}

pub fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    paired::ensure_powered::<MacOs>(config.power_on_adapter)?;
    paired::mark_search_starting()?;

    let inquiry = unsafe { IOBluetoothDeviceInquiry::inquiryWithDelegate(None) }
        .ok_or_else(|| anyhow!("macOS could not start a Bluetooth search"))?;
    unsafe {
        inquiry.setInquiryLength(SEARCH_SECONDS);
        inquiry.setUpdateNewDeviceNames(true);
    }
    let started = unsafe { inquiry.start() };
    if started != IO_RETURN_SUCCESS {
        paired::reset_discovery_state()?;
        bail!("macOS refused to start a Bluetooth search (code {started})");
    }

    let deadline = Instant::now() + Duration::from_secs(u64::from(SEARCH_SECONDS));
    while Instant::now() < deadline && paired::searching()? {
        deliver_pending_iobluetooth_callbacks(RUN_LOOP_SLICE);
        for device in readable_device_infos(unsafe { inquiry.foundDevices() }) {
            paired::record_discovered_device(device)?;
        }
    }
    unsafe { inquiry.stop() };
    paired::mark_search_stopped()?;

    let found = readable_device_infos(unsafe { inquiry.foundDevices() });
    qol_runtime::probe!("BLUETOOTH_SEARCH", "stage=stop found={}", found.len());
    Ok(found)
}

pub fn reconnect_devices(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    paired::reconnect_devices::<MacOs>(config, selection)
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    paired::devices_snapshot::<MacOs>()
}

pub fn run_daemon(config: ReconnectConfig) -> Result<()> {
    paired::run_daemon::<MacOs>(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dual_mode_device_listed_twice_keeps_its_richest_entry() {
        let bare = DeviceInfo {
            address: "AA:BB:CC:DD:EE:FF".into(),
            alias: "Jabra Evolve2 65 Flex".into(),
            paired: true,
            trusted: true,
            connected: false,
            audio_connected: None,
            services_resolved: false,
            icon: None,
            class: None,
            uuids: Vec::new(),
            rssi: None,
        };
        let connected = DeviceInfo {
            connected: true,
            uuids: vec!["0000110b-0000-1000-8000-00805f9b34fb".into()],
            ..bare.clone()
        };

        assert!(richer_duplicate(&bare, &connected));
        assert!(!richer_duplicate(&connected, &bare));
    }

    #[test]
    fn addresses_from_iobluetooth_reach_the_domain_in_colon_form() {
        let cases = [
            ("aa-bb-cc-dd-ee-ff", "AA:BB:CC:DD:EE:FF"),
            ("AA:BB:CC:DD:EE:FF", "AA:BB:CC:DD:EE:FF"),
        ];
        for (raw, expected) in cases {
            assert_eq!(colon_separated_address(raw), expected, "{raw}");
            assert_eq!(
                normalize_address(&colon_separated_address(raw)).unwrap(),
                expected
            );
        }
    }
}
