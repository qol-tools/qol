use std::mem::{offset_of, size_of, size_of_val, zeroed};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
    HidD_GetProductString, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS,
};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::UI::Input::XboxController::{
    XInputGetState, XINPUT_GAMEPAD, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK,
    XINPUT_GAMEPAD_BUTTON_FLAGS, XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT,
    XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER,
    XINPUT_GAMEPAD_LEFT_THUMB, XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB,
    XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE, XUSER_MAX_COUNT,
};

use crate::detection::clash::LinkEvidence;
use crate::fixes::DetectedDevice;
use crate::platform::{
    NativeButtonInput, NativeConnection, NativeControllerInput, NativeGamepadAxis,
    NativeGamepadButton, NativeGamepadState, NativeInputSnapshot, PlatformSupport,
};

const GENERIC_DESKTOP_PAGE: u16 = 0x01;
const GAME_CONTROLLER_USAGES: [u16; 3] = [0x04, 0x05, 0x08];
const BLUETOOTH_BUS: u16 = 0x0005;
const USB_BUS: u16 = 0x0003;
const OTHER_BUS: u16 = 0x0000;
const BLUETOOTH_HID_SERVICES: [&str; 2] = [
    "{00001124-0000-1000-8000-00805f9b34fb}",
    "{00001812-0000-1000-8000-00805f9b34fb}",
];
const PRODUCT_STRING_UNITS: usize = 256;
const INVALID_DEVICE_INFO_SET: HDEVINFO = -1;
const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const IDLE_SLOT_PROBE_INTERVAL: Duration = Duration::from_secs(1);
const XINPUT_SLOT_COUNT: usize = XUSER_MAX_COUNT as usize;
const STICK_RANGE: f32 = 32767.0;
const TRIGGER_RANGE: f32 = 255.0;
const PRESS_THRESHOLD: f32 = 0.05;
const LEFT_STICK_BUTTON: usize = 10;
const RIGHT_STICK_BUTTON: usize = 11;
const FALLBACK_XINPUT_NAME: &str = "XInput controller";

pub(crate) fn platform_support() -> PlatformSupport {
    PlatformSupport {
        label: "Windows",
        supported: true,
    }
}

#[derive(Default)]
pub struct InputMonitor {
    identities: Vec<HidController>,
    refreshed_at: Option<Instant>,
    connected: [bool; XINPUT_SLOT_COUNT],
    probed_at: Option<Instant>,
}

impl InputMonitor {
    pub fn snapshot(&mut self) -> NativeInputSnapshot {
        if self
            .refreshed_at
            .is_none_or(|time| time.elapsed() >= DEVICE_REFRESH_INTERVAL)
        {
            self.identities = game_controllers()
                .into_iter()
                .filter(|controller| controller.xinput)
                .collect();
            self.refreshed_at = Some(Instant::now());
        }
        let probe_idle = self
            .probed_at
            .is_none_or(|time| time.elapsed() >= IDLE_SLOT_PROBE_INTERVAL);
        if probe_idle {
            self.probed_at = Some(Instant::now());
        }
        let mut pads = Vec::new();
        for (slot, connected) in self.connected.iter_mut().enumerate() {
            if !*connected && !probe_idle {
                continue;
            }
            let pad = xinput_gamepad(slot as u32);
            *connected = pad.is_some();
            pads.extend(pad);
        }
        let items = pads
            .iter()
            .enumerate()
            .map(|(position, pad)| controller_input(self.identities.get(position), pad))
            .collect();
        NativeInputSnapshot {
            available: true,
            source: Some("windows-xinput"),
            items,
        }
    }
}

pub fn read_devices() -> Vec<DetectedDevice> {
    game_controllers()
        .iter()
        .map(HidController::detected_device)
        .collect()
}

pub fn link_evidence(devices: &[DetectedDevice]) -> Vec<Option<LinkEvidence>> {
    vec![None; devices.len()]
}

pub fn disconnect_bluetooth(_device: &DetectedDevice) -> Result<String> {
    bail!(
        "reconnecting a stuck controller needs BlueZ on Linux; Windows has no API that drops one Bluetooth link without unpairing the controller"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Transport {
    Bluetooth,
    Usb,
    Other,
}

impl Transport {
    fn key(self) -> &'static str {
        match self {
            Self::Bluetooth => "bluetooth",
            Self::Usb => "usb",
            Self::Other => "other",
        }
    }

    fn bus(self) -> u16 {
        match self {
            Self::Bluetooth => BLUETOOTH_BUS,
            Self::Usb => USB_BUS,
            Self::Other => OTHER_BUS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HidController {
    path: String,
    name: String,
    vendor: u16,
    product: u16,
    version: u16,
    transport: Transport,
    xinput: bool,
}

impl HidController {
    fn detected_device(&self) -> DetectedDevice {
        DetectedDevice {
            bus: self.transport.bus(),
            vendor: self.vendor,
            product: self.product,
            version: self.version,
            name: self.name.clone(),
            uniq: None,
            sysfs_path: None,
            event_handler: None,
            driver: Some(driver_name(self.xinput).to_string()),
            is_gamepad: true,
            has_force_feedback: self.xinput,
        }
    }
}

fn game_controllers() -> Vec<HidController> {
    let mut controllers = interface_paths()
        .into_iter()
        .filter_map(|path| read_controller(&path))
        .collect::<Vec<_>>();
    controllers.sort_by(|left, right| left.path.cmp(&right.path));
    controllers
}

fn read_controller(path: &str) -> Option<HidController> {
    let device = DeviceHandle::open(path)?;
    let (usage_page, usage) = device.top_level_usage()?;
    if !is_game_controller_usage(usage_page, usage) {
        return None;
    }
    let attributes = device.attributes()?;
    let name = device
        .product_string()
        .unwrap_or_else(|| fallback_name(attributes.VendorID, attributes.ProductID));
    Some(HidController {
        path: path.to_string(),
        name,
        vendor: attributes.VendorID,
        product: attributes.ProductID,
        version: attributes.VersionNumber,
        transport: transport_from_path(path),
        xinput: is_xinput_path(path),
    })
}

fn interface_paths() -> Vec<String> {
    let mut guid: GUID = unsafe { zeroed() };
    unsafe { HidD_GetHidGuid(&mut guid) };
    let set = unsafe {
        SetupDiGetClassDevsW(
            &guid,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_DEVICE_INFO_SET {
        return Vec::new();
    }
    let set = DeviceInfoSet(set);
    let mut paths = Vec::new();
    for index in 0.. {
        let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { zeroed() };
        interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        let found =
            unsafe { SetupDiEnumDeviceInterfaces(set.0, null(), &guid, index, &mut interface) };
        if found == 0 {
            break;
        }
        paths.extend(interface_path(set.0, &interface));
    }
    paths
}

fn interface_path(set: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    let mut required = 0u32;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(set, interface, null_mut(), 0, &mut required, null_mut())
    };
    let header = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
    let required_bytes = required as usize;
    if required_bytes <= offset {
        return None;
    }
    let mut buffer = vec![0u64; required_bytes.div_ceil(size_of::<u64>())];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    unsafe { std::ptr::addr_of_mut!((*detail).cbSize).write_unaligned(header as u32) };
    let filled = unsafe {
        SetupDiGetDeviceInterfaceDetailW(set, interface, detail, required, null_mut(), null_mut())
    };
    if filled == 0 {
        return None;
    }
    let units = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>().add(offset).cast::<u16>(),
            (required_bytes - offset) / size_of::<u16>(),
        )
    };
    Some(wide_to_string(units)).filter(|path| !path.is_empty())
}

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

struct DeviceHandle(HANDLE);

impl DeviceHandle {
    fn open(path: &str) -> Option<Self> {
        let wide = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        (handle != INVALID_HANDLE_VALUE && !handle.is_null()).then_some(Self(handle))
    }

    fn top_level_usage(&self) -> Option<(u16, u16)> {
        let mut preparsed = 0;
        if unsafe { HidD_GetPreparsedData(self.0, &mut preparsed) } == 0 {
            return None;
        }
        let mut caps: HIDP_CAPS = unsafe { zeroed() };
        let status = unsafe { HidP_GetCaps(preparsed, &mut caps) };
        unsafe { HidD_FreePreparsedData(preparsed) };
        (status == HIDP_STATUS_SUCCESS).then_some((caps.UsagePage, caps.Usage))
    }

    fn attributes(&self) -> Option<HIDD_ATTRIBUTES> {
        let mut attributes: HIDD_ATTRIBUTES = unsafe { zeroed() };
        attributes.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
        (unsafe { HidD_GetAttributes(self.0, &mut attributes) } != 0).then_some(attributes)
    }

    fn product_string(&self) -> Option<String> {
        let mut buffer = [0u16; PRODUCT_STRING_UNITS];
        let read = unsafe {
            HidD_GetProductString(
                self.0,
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
            )
        };
        if read == 0 {
            return None;
        }
        let name = wide_to_string(&buffer).trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

impl Drop for DeviceHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn xinput_gamepad(slot: u32) -> Option<XINPUT_GAMEPAD> {
    let mut state: XINPUT_STATE = unsafe { zeroed() };
    (unsafe { XInputGetState(slot, &mut state) } == ERROR_SUCCESS).then_some(state.Gamepad)
}

fn controller_input(
    identity: Option<&HidController>,
    pad: &XINPUT_GAMEPAD,
) -> NativeControllerInput {
    NativeControllerInput {
        name: identity.map_or_else(
            || FALLBACK_XINPUT_NAME.to_string(),
            |identity| identity.name.clone(),
        ),
        vendor: identity.map_or(0, |identity| identity.vendor),
        product: identity.map_or(0, |identity| identity.product),
        connection: NativeConnection {
            transport: identity
                .map_or(Transport::Other, |identity| identity.transport)
                .key(),
            signal: None,
            adapter: None,
        },
        buttons: vec![
            NativeButtonInput {
                index: LEFT_STICK_BUTTON,
                pressed: held(pad.wButtons, XINPUT_GAMEPAD_LEFT_THUMB),
            },
            NativeButtonInput {
                index: RIGHT_STICK_BUTTON,
                pressed: held(pad.wButtons, XINPUT_GAMEPAD_RIGHT_THUMB),
            },
        ],
        state: gamepad_state(pad),
    }
}

fn gamepad_state(pad: &XINPUT_GAMEPAD) -> NativeGamepadState {
    let digital = |flag| f32::from(u8::from(held(pad.wButtons, flag)));
    let specs = [
        (0, "South", digital(XINPUT_GAMEPAD_A)),
        (1, "East", digital(XINPUT_GAMEPAD_B)),
        (2, "West", digital(XINPUT_GAMEPAD_X)),
        (3, "North", digital(XINPUT_GAMEPAD_Y)),
        (4, "Left shoulder", digital(XINPUT_GAMEPAD_LEFT_SHOULDER)),
        (5, "Right shoulder", digital(XINPUT_GAMEPAD_RIGHT_SHOULDER)),
        (6, "Left trigger", trigger(pad.bLeftTrigger)),
        (7, "Right trigger", trigger(pad.bRightTrigger)),
        (8, "Select", digital(XINPUT_GAMEPAD_BACK)),
        (9, "Start", digital(XINPUT_GAMEPAD_START)),
        (10, "Left stick", digital(XINPUT_GAMEPAD_LEFT_THUMB)),
        (11, "Right stick", digital(XINPUT_GAMEPAD_RIGHT_THUMB)),
        (12, "D-pad up", digital(XINPUT_GAMEPAD_DPAD_UP)),
        (13, "D-pad down", digital(XINPUT_GAMEPAD_DPAD_DOWN)),
        (14, "D-pad left", digital(XINPUT_GAMEPAD_DPAD_LEFT)),
        (15, "D-pad right", digital(XINPUT_GAMEPAD_DPAD_RIGHT)),
    ];
    let buttons = specs
        .into_iter()
        .map(|(index, name, value)| NativeGamepadButton {
            index,
            name,
            pressed: value > PRESS_THRESHOLD,
            value,
        })
        .collect();
    let axes = [
        (0, "Left X", stick(pad.sThumbLX)),
        (1, "Left Y", -stick(pad.sThumbLY)),
        (2, "Right X", stick(pad.sThumbRX)),
        (3, "Right Y", -stick(pad.sThumbRY)),
    ]
    .into_iter()
    .map(|(index, name, value)| NativeGamepadAxis { index, name, value })
    .collect();
    NativeGamepadState {
        mapping: "standard",
        buttons,
        axes,
    }
}

fn held(buttons: XINPUT_GAMEPAD_BUTTON_FLAGS, flag: XINPUT_GAMEPAD_BUTTON_FLAGS) -> bool {
    buttons & flag != 0
}

fn trigger(value: u8) -> f32 {
    f32::from(value) / TRIGGER_RANGE
}

fn stick(value: i16) -> f32 {
    (f32::from(value) / STICK_RANGE).clamp(-1.0, 1.0)
}

fn wide_to_string(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn is_game_controller_usage(usage_page: u16, usage: u16) -> bool {
    usage_page == GENERIC_DESKTOP_PAGE && GAME_CONTROLLER_USAGES.contains(&usage)
}

fn transport_from_path(path: &str) -> Transport {
    let path = path.to_ascii_lowercase();
    if path.contains("bthledevice")
        || BLUETOOTH_HID_SERVICES
            .iter()
            .any(|service| path.contains(service))
    {
        return Transport::Bluetooth;
    }
    if path.contains("hid#vid_") {
        return Transport::Usb;
    }
    Transport::Other
}

fn is_xinput_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("&ig_")
}

fn driver_name(xinput: bool) -> &'static str {
    if xinput {
        "xinput"
    } else {
        "hid"
    }
}

fn fallback_name(vendor: u16, product: u16) -> String {
    format!("Game controller {vendor:04x}:{product:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    type StateCase = (XINPUT_GAMEPAD, &'static [(usize, f32)], [f32; 4]);

    #[test]
    fn device_paths_classify_transport_and_xinput() {
        let cases = [
            (
                r"\\?\hid#vid_045e&pid_028e&ig_00#8&2b1b7c0b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
                Transport::Usb,
                true,
            ),
            (
                r"\\?\HID#VID_054C&PID_0CE6&MI_03#7&1a2b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
                Transport::Usb,
                false,
            ),
            (
                r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#9&1f1e&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
                Transport::Bluetooth,
                false,
            ),
            (
                r"\\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02045e_pid&0b13_rev&0517_7c1b&ig_00#a&2a&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
                Transport::Bluetooth,
                true,
            ),
            (
                r"\\?\hid#hidclass&col01#1&2d595ca7&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}",
                Transport::Other,
                false,
            ),
        ];
        for (path, transport, xinput) in cases {
            assert_eq!(transport_from_path(path), transport, "path={path}");
            assert_eq!(is_xinput_path(path), xinput, "path={path}");
        }
    }

    #[test]
    fn only_generic_desktop_controller_collections_count() {
        let cases = [
            (0x01, 0x04, true),
            (0x01, 0x05, true),
            (0x01, 0x08, true),
            (0x01, 0x02, false),
            (0x01, 0x06, false),
            (0x0c, 0x05, false),
        ];
        for (page, usage, expected) in cases {
            assert_eq!(
                is_game_controller_usage(page, usage),
                expected,
                "page={page:#x} usage={usage:#x}"
            );
        }
    }

    #[test]
    fn controllers_map_to_detected_devices() {
        let cases = [
            (
                Transport::Bluetooth,
                true,
                BLUETOOTH_BUS,
                "Bluetooth",
                "xinput",
            ),
            (Transport::Usb, false, USB_BUS, "USB", "hid"),
            (Transport::Other, false, OTHER_BUS, "Other", "hid"),
        ];
        for (transport, xinput, bus, label, driver) in cases {
            let device = controller("Pad", transport, xinput).detected_device();
            assert_eq!(device.bus, bus);
            assert_eq!(device.transport(), label);
            assert_eq!(device.driver_label(), driver);
            assert_eq!(device.has_force_feedback, xinput);
            assert!(device.is_gamepad);
            assert!(!device.is_virtual());
        }
    }

    #[test]
    fn xinput_state_follows_the_standard_gamepad_mapping() {
        let cases: [StateCase; 4] = [
            (pad(0, 0, 0, [0, 0, 0, 0]), &[], [0.0, 0.0, 0.0, 0.0]),
            (
                pad(XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_START, 0, 0, [0, 0, 0, 0]),
                &[(0, 1.0), (9, 1.0)],
                [0.0, 0.0, 0.0, 0.0],
            ),
            (
                pad(
                    XINPUT_GAMEPAD_DPAD_LEFT | XINPUT_GAMEPAD_RIGHT_THUMB,
                    255,
                    0,
                    [0, 0, 0, 0],
                ),
                &[(6, 1.0), (11, 1.0), (14, 1.0)],
                [0.0, 0.0, 0.0, 0.0],
            ),
            (
                pad(0, 0, 0, [i16::MAX, i16::MAX, i16::MIN, i16::MIN]),
                &[],
                [1.0, -1.0, -1.0, 1.0],
            ),
        ];
        for (input, pressed, axes) in cases {
            let state = gamepad_state(&input);
            assert_eq!(state.mapping, "standard");
            assert_eq!(state.buttons.len(), 16);
            for button in &state.buttons {
                let expected = pressed
                    .iter()
                    .find(|(index, _)| *index == button.index)
                    .map_or(0.0, |(_, value)| *value);
                assert_eq!(button.value, expected, "button {}", button.name);
                assert_eq!(button.pressed, expected > PRESS_THRESHOLD);
            }
            let values = state.axes.iter().map(|axis| axis.value).collect::<Vec<_>>();
            assert_eq!(values, axes);
        }
    }

    #[test]
    fn xinput_items_borrow_the_matching_hid_identity() {
        let identity = controller("Controller (Xbox One For Windows)", Transport::Usb, true);
        let stick_pressed = pad(XINPUT_GAMEPAD_LEFT_THUMB, 0, 0, [0, 0, 0, 0]);
        let cases = [
            (
                Some(&identity),
                "Controller (Xbox One For Windows)",
                0x045e,
                "usb",
            ),
            (None, FALLBACK_XINPUT_NAME, 0, "other"),
        ];
        for (identity, name, vendor, transport) in cases {
            let item = controller_input(identity, &stick_pressed);
            assert_eq!(item.name, name);
            assert_eq!(item.vendor, vendor);
            assert_eq!(item.connection.transport, transport);
            let overrides = item
                .buttons
                .iter()
                .map(|button| (button.index, button.pressed))
                .collect::<Vec<_>>();
            assert_eq!(
                overrides,
                [(LEFT_STICK_BUTTON, true), (RIGHT_STICK_BUTTON, false)]
            );
        }
    }

    #[test]
    fn wide_strings_stop_at_the_first_nul() {
        let cases: [(&[u16], &str); 3] = [
            (&[0x50, 0x61, 0x64, 0, 0x58], "Pad"),
            (&[0x50, 0x61, 0x64], "Pad"),
            (&[0], ""),
        ];
        for (units, expected) in cases {
            assert_eq!(wide_to_string(units), expected);
        }
    }

    fn controller(name: &str, transport: Transport, xinput: bool) -> HidController {
        HidController {
            path: String::new(),
            name: name.to_string(),
            vendor: 0x045e,
            product: 0x0b13,
            version: 0x0517,
            transport,
            xinput,
        }
    }

    fn pad(
        buttons: XINPUT_GAMEPAD_BUTTON_FLAGS,
        left_trigger: u8,
        right_trigger: u8,
        sticks: [i16; 4],
    ) -> XINPUT_GAMEPAD {
        XINPUT_GAMEPAD {
            wButtons: buttons,
            bLeftTrigger: left_trigger,
            bRightTrigger: right_trigger,
            sThumbLX: sticks[0],
            sThumbLY: sticks[1],
            sThumbRX: sticks[2],
            sThumbRY: sticks[3],
        }
    }
}
