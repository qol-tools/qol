use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::CString;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_bluetooth::{
    CBAdvertisementDataIsConnectable, CBAdvertisementDataLocalNameKey,
    CBAdvertisementDataServiceUUIDsKey, CBCentralManager, CBCentralManagerDelegate,
    CBCentralManagerScanOptionAllowDuplicatesKey, CBManager, CBManagerAuthorization,
    CBManagerState, CBPeripheral, CBUUID,
};
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use objc2_io_bluetooth::{
    BluetoothHCIPowerState, IOBluetoothDevice, IOBluetoothDeviceInquiry, IOBluetoothDevicePair,
    IOBluetoothHostController, IOBluetoothSDPUUID,
};
use qol_headless::DoctorCheckResult;

use crate::bluetooth::{
    normalize_address, AdapterHealth, BackendCapabilities, DeviceInfo, Pairing, ReconnectReport,
    ReconnectSelection,
};
use crate::config::ReconnectConfig;

use super::nearby::{self, Advertisement, Sightings};
use super::paired::{self, PairedStack};
pub use super::paired::{search_status_snapshot, settings_action, settings_query, stop_search};

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
    audio_reclaim: crate::audio_claim::platform::RECLAIM_SUPPORTED,
    settings_pairing: Some("System Settings"),
};

const AUDIO_SINK_UUID16: u16 = 0x110b;
const AUDIO_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const IO_RETURN_SUCCESS: i32 = 0;
const RSSI_UNAVAILABLE: i8 = 127;
const RSSI_UNTRACKED: i8 = 0;
const SEARCH_SECONDS: u8 = 10;
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_LOOP_SLICE: Duration = Duration::from_millis(100);
const ADAPTER_POWER_SYMBOL: &str = "IOBluetoothPreferenceSetControllerPowerState";
const TRUST_UNSUPPORTED: &str =
    "macOS pairs and trusts in one step, so there is no separate trust to change";
const BLUETOOTH_PRIVACY_SETTINGS: &str = "System Settings > Privacy & Security > Bluetooth";
const LOW_ENERGY_LEFT_OUT: &str = "Bluetooth LE devices are left out of this search because";

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

    fn reconnect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
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

    fn on_stop_request(_stop: impl FnOnce() + Send + 'static) {}
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

fn device_info(device: &IOBluetoothDevice, paired: &HashSet<String>) -> Result<DeviceInfo> {
    let address = device_address(device)?;
    let alias = device_alias(device, &address);
    let paired = paired.contains(&address) || unsafe { device.isPaired() };
    let (services_resolved, uuids) = cached_audio_sink_service(device);
    let connected = unsafe { device.isConnected() };
    Ok(DeviceInfo {
        alias,
        trusted: paired,
        paired,
        connected,
        audio_connected: None,
        services_resolved,
        icon: None,
        class: Some(unsafe { device.classOfDevice() }),
        uuids,
        rssi: iobluetooth_signal(unsafe { device.rawRSSI() }),
        pairing: Pairing::Backend,
        address,
    })
}

fn iobluetooth_signal(raw_rssi: i8) -> Option<i16> {
    match raw_rssi {
        RSSI_UNAVAILABLE | RSSI_UNTRACKED => None,
        measured => Some(i16::from(measured)),
    }
}

fn iobluetooth_devices(devices: Option<Retained<NSArray>>) -> Vec<Retained<IOBluetoothDevice>> {
    let Some(devices) = devices else {
        return Vec::new();
    };
    devices
        .iter()
        .map(|object| unsafe { Retained::cast_unchecked::<IOBluetoothDevice>(object) })
        .collect()
}

fn addresses(devices: &[Retained<IOBluetoothDevice>]) -> HashSet<String> {
    devices
        .iter()
        .filter_map(|device| device_address(device).ok())
        .collect()
}

fn readable_device_infos(
    devices: &[Retained<IOBluetoothDevice>],
    paired: &HashSet<String>,
) -> Vec<DeviceInfo> {
    nearby::unique(
        devices
            .iter()
            .filter_map(|device| device_info(device, paired).ok()),
    )
}

fn paired_addresses() -> HashSet<String> {
    addresses(&iobluetooth_devices(unsafe {
        IOBluetoothDevice::pairedDevices()
    }))
}

fn paired_devices() -> Vec<DeviceInfo> {
    let devices = iobluetooth_devices(unsafe { IOBluetoothDevice::pairedDevices() });
    readable_device_infos(&devices, &addresses(&devices))
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Sightings>]
    struct LowEnergyScanner;

    unsafe impl NSObjectProtocol for LowEnergyScanner {}

    unsafe impl CBCentralManagerDelegate for LowEnergyScanner {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn central_state_changed(&self, _central: &CBCentralManager) {}

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn peripheral_advertised(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            advertisement_data: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            let sighting = advertisement(peripheral, advertisement_data, rssi);
            self.ivars().borrow_mut().record(sighting);
        }
    }
);

impl LowEnergyScanner {
    fn new(marker: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(RefCell::new(Sightings::default()));
        unsafe { msg_send![super(this), init] }
    }
}

struct LowEnergyScan {
    central: Retained<CBCentralManager>,
    scanner: Retained<LowEnergyScanner>,
    scanning: bool,
}

impl LowEnergyScan {
    fn begin() -> Option<Self> {
        let Some(marker) = MainThreadMarker::new() else {
            log::warn!("{LOW_ENERGY_LEFT_OUT} it is not running on the main thread");
            return None;
        };
        if low_energy_denied(unsafe { CBManager::authorization_class() }) {
            log::warn!("{LOW_ENERGY_LEFT_OUT} {}", access_denied());
            return None;
        }
        let scanner = LowEnergyScanner::new(marker);
        let central = unsafe { CBCentralManager::new() };
        unsafe { central.setDelegate(Some(ProtocolObject::from_ref(&*scanner))) };
        Some(Self {
            central,
            scanner,
            scanning: false,
        })
    }

    fn scan_once_powered(&mut self) {
        if self.scanning || unsafe { self.central.state() } != CBManagerState::PoweredOn {
            return;
        }
        let allow_duplicates = NSNumber::new_bool(true);
        let value: &AnyObject = &allow_duplicates;
        let options = NSDictionary::from_slices(
            &[unsafe { CBCentralManagerScanOptionAllowDuplicatesKey }],
            &[value],
        );
        unsafe {
            self.central
                .scanForPeripheralsWithServices_options(None, Some(&options))
        };
        self.scanning = true;
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.scanner.ivars().borrow().devices()
    }

    fn report_unscanned(&self) {
        if self.scanning {
            return;
        }
        let state = unsafe { self.central.state() };
        log::warn!("{LOW_ENERGY_LEFT_OUT} {}", unready_reason(state));
    }
}

impl Drop for LowEnergyScan {
    fn drop(&mut self) {
        if self.scanning {
            unsafe { self.central.stopScan() };
        }
        unsafe { self.central.setDelegate(None) };
    }
}

fn low_energy_denied(authorization: CBManagerAuthorization) -> bool {
    authorization == CBManagerAuthorization::Denied
        || authorization == CBManagerAuthorization::Restricted
}

fn access_denied() -> String {
    format!("macOS denies this app Bluetooth access; allow it in {BLUETOOTH_PRIVACY_SETTINGS}")
}

fn unready_reason(state: CBManagerState) -> String {
    match state {
        CBManagerState::Unauthorized => access_denied(),
        CBManagerState::PoweredOff => "CoreBluetooth reports Bluetooth as off".into(),
        CBManagerState::Unsupported => "this Mac does not support Bluetooth LE".into(),
        other => format!("CoreBluetooth did not become ready (state {})", other.0),
    }
}

fn advertisement(
    peripheral: &CBPeripheral,
    data: &NSDictionary<NSString, AnyObject>,
    rssi: &NSNumber,
) -> Advertisement {
    let local_name = data
        .objectForKey(unsafe { CBAdvertisementDataLocalNameKey })
        .and_then(|name| name.downcast::<NSString>().ok())
        .map(|name| name.to_string());
    let connectable = data
        .objectForKey(unsafe { CBAdvertisementDataIsConnectable })
        .and_then(|flag| flag.downcast::<NSNumber>().ok())
        .is_some_and(|flag| flag.as_bool());
    let service_uuids = data
        .objectForKey(unsafe { CBAdvertisementDataServiceUUIDsKey })
        .and_then(|uuids| uuids.downcast::<NSArray>().ok())
        .map(|uuids| advertised_services(&uuids))
        .unwrap_or_default();
    Advertisement {
        identifier: unsafe { peripheral.identifier() }.UUIDString().to_string(),
        local_name,
        peripheral_name: unsafe { peripheral.name() }.map(|name| name.to_string()),
        rssi: rssi.as_i64(),
        connectable,
        service_uuids,
    }
}

fn advertised_services(uuids: &NSArray) -> Vec<String> {
    uuids
        .iter()
        .filter_map(|uuid| uuid.downcast::<CBUUID>().ok())
        .map(|uuid| unsafe { uuid.UUIDString() }.to_string())
        .collect()
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
            "platform": qol_platform::Os::Macos.name(),
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
        return device_info(&device, &paired_addresses());
    }
    let result = unsafe { device.openConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not connect to {address} (code {result})");
    }
    if !deliver_callbacks_until(CONNECT_TIMEOUT, || unsafe { device.isConnected() }) {
        bail!("{address} did not finish connecting");
    }
    device_info(&device, &paired_addresses())
}

pub fn disconnect_device(address: &str) -> Result<DeviceInfo> {
    let device = device_handle(address)?;
    let result = unsafe { device.closeConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not disconnect {address} (code {result})");
    }
    device_info(&device, &paired_addresses())
}

pub fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    paired::ensure_powered::<MacOs>(power_on_adapter)?;
    let device = device_handle(address)?;
    let known = device_info(&device, &paired_addresses())?;
    if known.paired {
        return Ok(known);
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
    device_info(&device, &paired_addresses())
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

    let mut low_energy = LowEnergyScan::begin();
    let watched = watch_search(&inquiry, low_energy.as_mut());
    unsafe { inquiry.stop() };
    let found = found_devices(&inquiry, low_energy.as_ref());
    if let Some(scan) = &low_energy {
        scan.report_unscanned();
    }
    drop(low_energy);
    paired::mark_search_stopped()?;
    watched?;

    let devices = nearby::merge(paired_devices().into_iter().chain(found));
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH",
        "stage=stop devices={} low_energy={}",
        devices.len(),
        devices
            .iter()
            .filter(|device| device.pairing == Pairing::SystemSettings)
            .count()
    );
    Ok(devices)
}

fn watch_search(
    inquiry: &IOBluetoothDeviceInquiry,
    mut low_energy: Option<&mut LowEnergyScan>,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(u64::from(SEARCH_SECONDS));
    while Instant::now() < deadline && paired::searching()? {
        deliver_pending_iobluetooth_callbacks(RUN_LOOP_SLICE);
        if let Some(scan) = low_energy.as_deref_mut() {
            scan.scan_once_powered();
        }
        for device in found_devices(inquiry, low_energy.as_deref()) {
            paired::record_discovered_device(device)?;
        }
    }
    Ok(())
}

fn found_devices(
    inquiry: &IOBluetoothDeviceInquiry,
    low_energy: Option<&LowEnergyScan>,
) -> Vec<DeviceInfo> {
    let classic = iobluetooth_devices(unsafe { inquiry.foundDevices() });
    let mut found = readable_device_infos(&classic, &paired_addresses());
    found.extend(low_energy.map(LowEnergyScan::devices).unwrap_or_default());
    found
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

    #[test]
    fn a_signal_iobluetooth_could_not_read_is_reported_as_unknown() {
        let cases = [
            (-48, Some(-48)),
            (-61, Some(-61)),
            (RSSI_UNTRACKED, None),
            (RSSI_UNAVAILABLE, None),
        ];
        for (raw_rssi, expected) in cases {
            assert_eq!(iobluetooth_signal(raw_rssi), expected, "{raw_rssi}");
        }
    }
}
