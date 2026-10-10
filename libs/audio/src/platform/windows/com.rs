use std::ffi::c_void;

use qol_platform::native::com::{Apartment, ComApartment};
use qol_platform::native::wide::wide_nul;
use windows::core::{Error, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_ContainerId;
use windows::Win32::Foundation::{ERROR_NOT_FOUND, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IConnector, IDeviceTopology, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Variant::{VARENUM, VT_CLSID, VT_LPWSTR, VT_UI4};

use crate::devices::{Direction, Identity};
use crate::AudioError;

pub(super) fn apartment() -> Result<ComApartment, AudioError> {
    ComApartment::enter(Apartment::MultiThreaded)
        .map_err(|error| AudioError::ServerUnavailable(format!("COM did not start: {error}")))
}

pub(super) struct Endpoint {
    pub(super) id: String,
    pub(super) device: IMMDevice,
}

pub(super) fn with_enumerator<T>(
    operation: impl FnOnce(&IMMDeviceEnumerator) -> Result<T, AudioError>,
) -> Result<T, AudioError> {
    let _com = apartment()?;
    let enumerator = enumerator()?;
    operation(&enumerator)
}

pub(super) fn enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|error| {
        AudioError::ServerUnavailable(format!("the Windows audio service did not answer: {error}"))
    })
}

pub(super) fn failed(context: &'static str) -> impl Fn(Error) -> AudioError {
    move |error| AudioError::Operation(format!("{context}: {error}"))
}

fn flow(direction: Direction) -> EDataFlow {
    match direction {
        Direction::Output => eRender,
        Direction::Input => eCapture,
    }
}

pub(super) fn endpoints(
    enumerator: &IMMDeviceEnumerator,
    direction: Direction,
    states: DEVICE_STATE,
) -> Result<Vec<Endpoint>, AudioError> {
    let collection = unsafe { enumerator.EnumAudioEndpoints(flow(direction), states) }
        .map_err(failed("cannot list the audio endpoints"))?;
    let count =
        unsafe { collection.GetCount() }.map_err(failed("cannot count the audio endpoints"))?;
    (0..count)
        .map(|index| {
            let device = unsafe { collection.Item(index) }
                .map_err(failed("cannot read an audio endpoint"))?;
            Ok(Endpoint {
                id: id_of(&device)?,
                device,
            })
        })
        .collect()
}

pub(super) fn default_endpoint(
    enumerator: &IMMDeviceEnumerator,
    direction: Direction,
) -> Result<Option<Endpoint>, AudioError> {
    match unsafe { enumerator.GetDefaultAudioEndpoint(flow(direction), eConsole) } {
        Ok(device) => Ok(Some(Endpoint {
            id: id_of(&device)?,
            device,
        })),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) => Ok(None),
        Err(error) => Err(failed("cannot read the default audio endpoint")(error)),
    }
}

pub(super) fn endpoint(
    enumerator: &IMMDeviceEnumerator,
    direction: Direction,
    id: &str,
) -> Result<Endpoint, AudioError> {
    let wide = wide_nul(id);
    match unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) } {
        Ok(device) => Ok(Endpoint {
            id: id_of(&device)?,
            device,
        }),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) => {
            Err(AudioError::not_present(direction, &Identity::from_raw(id)))
        }
        Err(error) => Err(failed("cannot open the audio endpoint")(error)),
    }
}

fn id_of(device: &IMMDevice) -> Result<String, AudioError> {
    let id = unsafe { device.GetId() }.map_err(failed("cannot read an audio endpoint id"))?;
    Ok(owned_text(id))
}

pub(super) fn owned_text(text: PWSTR) -> String {
    let value = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as *const c_void)) };
    value
}

fn property(device: &IMMDevice, key: &PROPERTYKEY, kind: VARENUM) -> Option<PROPVARIANT> {
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    let value = unsafe { store.GetValue(key) }.ok()?;
    (value.vt() == kind).then_some(value)
}

pub(super) fn text_property(device: &IMMDevice, key: &PROPERTYKEY) -> Option<String> {
    let value = property(device, key, VT_LPWSTR)?;
    let text = unsafe { value.Anonymous.Anonymous.Anonymous.pwszVal.to_string() }.ok()?;
    (!text.trim().is_empty()).then_some(text)
}

pub(super) fn number_property(device: &IMMDevice, key: &PROPERTYKEY) -> Option<u32> {
    let value = property(device, key, VT_UI4)?;
    Some(unsafe { value.Anonymous.Anonymous.Anonymous.ulVal })
}

pub(super) fn container(device: &IMMDevice) -> Option<GUID> {
    let value = property(device, &PKEY_Device_ContainerId, VT_CLSID)?;
    unsafe { value.Anonymous.Anonymous.Anonymous.puuid.as_ref() }.copied()
}

pub(super) fn topology_link(device: &IMMDevice) -> Option<(String, IConnector)> {
    let topology: IDeviceTopology = unsafe { device.Activate(CLSCTX_ALL, None) }.ok()?;
    let connector = unsafe { topology.GetConnector(0) }.ok()?;
    let path = owned_text(unsafe { connector.GetDeviceIdConnectedTo() }.ok()?);
    Some((path, connector))
}
