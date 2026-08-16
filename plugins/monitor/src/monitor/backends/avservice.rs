use std::collections::HashSet;
use std::ffi::c_void;
use std::fmt;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use qol_windowing::DisplayEnumerator;

use crate::monitor::backends::i2c_ddc::{
    get_vcp_request, parse_get_vcp_reply, percent_from_raw, set_vcp_request, I2cError, REPLY_LEN,
    RESPONSE_DELAY, SETTLE_DELAY, WRITE_RETRIES,
};
use crate::monitor::policy::DdcStatus;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, GammaStateControl, HdrState, MonitorError, RestoreOutcome,
    GAMMA_REASON, HDR_REASON, MODES_REASON,
};
use crate::session::{LutProvider, LutRestoreOutcome};

const FEATURE_BRIGHTNESS: u8 = 0x10;
#[cfg(target_os = "macos")]
const I2C_BUS_ID: u32 = 0x37;
#[cfg(target_os = "macos")]
const I2C_DATA_ADDRESS: u32 = 0x51;

#[derive(Debug)]
pub enum AvError {
    MissingSymbol { name: String },
    CountMismatch { displays: usize, services: usize },
    Protocol { detail: String },
    Unsupported { detail: String },
}

impl AvError {
    fn surfaces_on_probe(&self) -> bool {
        matches!(
            self,
            Self::MissingSymbol { .. } | Self::CountMismatch { .. }
        )
    }
}

impl fmt::Display for AvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSymbol { name } => {
                write!(
                    f,
                    "the private IOKit symbol {name} is not exported by this macOS"
                )
            }
            Self::CountMismatch { displays, services } => write!(
                f,
                "{displays} external display(s) but {services} DCPAVServiceProxy service(s)"
            ),
            Self::Protocol { detail } => write!(f, "invalid DDC/CI exchange: {detail}"),
            Self::Unsupported { detail } => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for AvError {}

impl From<I2cError> for AvError {
    fn from(error: I2cError) -> Self {
        match error {
            I2cError::Protocol { detail } => Self::Protocol { detail },
            other => Self::Unsupported {
                detail: other.to_string(),
            },
        }
    }
}

impl From<MonitorError> for AvError {
    fn from(error: MonitorError) -> Self {
        match error {
            MonitorError::Unsupported { reason, .. } | MonitorError::Refused { reason, .. } => {
                Self::Unsupported { detail: reason }
            }
            other => Self::Unsupported {
                detail: other.to_string(),
            },
        }
    }
}

impl From<AvError> for MonitorError {
    fn from(error: AvError) -> Self {
        match error {
            AvError::MissingSymbol { name } => Self::unsupported(
                "brightness",
                format!("{name} is not exported by this macOS; IOAVService DDC is unavailable"),
            ),
            AvError::CountMismatch { displays, services } => Self::unsupported(
                "brightness",
                format!(
                    "{displays} external display(s) but {services} DCPAVServiceProxy service(s); \
                     refusing to guess the pairing"
                ),
            ),
            AvError::Protocol { detail } => Self::I2c(I2cError::Protocol { detail }),
            AvError::Unsupported { detail } => Self::unsupported("brightness", detail),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvService {
    raw: *mut c_void,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl AvService {
    fn new(raw: *mut c_void) -> Self {
        Self { raw }
    }

    fn raw(&self) -> *mut c_void {
        self.raw
    }
}

pub trait AvSymbolResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Result<*mut c_void, AvError>;
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct ResolvedSymbols<R: AvSymbolResolver> {
    resolver: R,
    cache: Mutex<Vec<(&'static str, usize)>>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl<R: AvSymbolResolver> ResolvedSymbols<R> {
    fn new(resolver: R) -> Self {
        Self {
            resolver,
            cache: Mutex::new(Vec::new()),
        }
    }

    fn get(&self, name: &'static str) -> Result<usize, AvError> {
        let mut cache = self.cache.lock().unwrap();
        if let Some((_, symbol)) = cache.iter().find(|(known, _)| *known == name) {
            return Ok(*symbol);
        }
        let symbol = self.resolver.resolve(name)? as usize;
        cache.push((name, symbol));
        Ok(symbol)
    }
}

pub trait AvTransport: Send + Sync {
    fn list_external_services(&self) -> Result<Vec<AvService>, AvError>;
    fn write(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError>;
    fn read(&self, service: &AvService, buffer: &mut [u8]) -> Result<(), AvError>;
}

fn platform_displays() -> Result<Vec<DisplayHandle>, MonitorError> {
    Ok(qol_windowing::Platform.enumerate()?)
}

pub struct MacAvServiceBackend<T: AvTransport> {
    transport: T,
    settle: Duration,
    response_delay: Duration,
    enumerate_displays: Box<dyn Fn() -> Result<Vec<DisplayHandle>, MonitorError> + Send + Sync>,
    dropped_writes: Mutex<HashSet<String>>,
}

impl<T: AvTransport> MacAvServiceBackend<T> {
    pub fn new(transport: T) -> Self {
        Self::with_timing(transport, SETTLE_DELAY, RESPONSE_DELAY)
    }

    pub fn with_timing(transport: T, settle: Duration, response_delay: Duration) -> Self {
        Self {
            transport,
            settle,
            response_delay,
            enumerate_displays: Box::new(platform_displays),
            dropped_writes: Mutex::new(HashSet::new()),
        }
    }

    fn writes_dropped(&self, connector: &str) -> bool {
        self.dropped_writes.lock().unwrap().contains(connector)
    }

    fn service_for<'a>(
        &self,
        handle: &DisplayHandle,
        services: &'a [AvService],
    ) -> Result<&'a AvService, AvError> {
        let displays = (self.enumerate_displays)()
            .map_err(AvError::from)?
            .into_iter()
            .filter(|display| !display.connector().ends_with("-builtin"))
            .collect::<Vec<_>>();
        if displays.len() != services.len() {
            return Err(AvError::CountMismatch {
                displays: displays.len(),
                services: services.len(),
            });
        }
        let position = displays
            .iter()
            .position(|display| display == handle)
            .ok_or_else(|| AvError::Unsupported {
                detail: format!("no enumerated display matches {}", handle.id()),
            })?;
        services.get(position).ok_or_else(|| AvError::Unsupported {
            detail: format!(
                "no DCPAVServiceProxy service pairs with {}",
                handle.connector()
            ),
        })
    }

    fn read_current_max(&self, service: &AvService) -> Result<(u16, u16), AvError> {
        let request = get_vcp_request(FEATURE_BRIGHTNESS);
        self.transport.write(service, &request[1..])?;
        thread::sleep(self.response_delay);
        let mut frame = [0u8; REPLY_LEN];
        self.transport.read(service, &mut frame)?;
        parse_get_vcp_reply(FEATURE_BRIGHTNESS, &frame).map_err(AvError::from)
    }

    fn read_back_matches(
        &self,
        service: &AvService,
        expected_percent: u8,
    ) -> Result<bool, AvError> {
        let (current, max) = self.read_current_max(service)?;
        Ok(percent_from_raw(current, max).map_err(AvError::from)? == expected_percent)
    }

    fn get_brightness_inner(&self, handle: &DisplayHandle) -> Result<u8, AvError> {
        let services = self.transport.list_external_services()?;
        let service = self.service_for(handle, &services)?;
        let (current, max) = self.read_current_max(service)?;
        percent_from_raw(current, max).map_err(AvError::from)
    }

    fn set_brightness_inner(&self, handle: &DisplayHandle, value: u8) -> Result<(), AvError> {
        let connector = handle.connector();
        if self.writes_dropped(connector) {
            return Err(AvError::Unsupported {
                detail: format!(
                    "DDC/CI writes were dropped on {connector}; set the display policy to gamma \
                     to keep brightness control"
                ),
            });
        }
        let services = self.transport.list_external_services()?;
        let service = self.service_for(handle, &services)?;
        let (_, max) = self.read_current_max(service)?;
        if max == 0 {
            return Err(AvError::Protocol {
                detail: format!("{connector} reports a maximum brightness of 0"),
            });
        }
        let target = (u32::from(value) * u32::from(max) / 100) as u16;
        let mut verified = false;
        for _ in 0..=WRITE_RETRIES {
            let request = set_vcp_request(FEATURE_BRIGHTNESS, target);
            self.transport.write(service, &request[1..])?;
            thread::sleep(self.settle);
            if self.read_back_matches(service, value)? {
                verified = true;
                break;
            }
        }
        if !verified {
            self.dropped_writes
                .lock()
                .unwrap()
                .insert(connector.to_string());
        }
        Ok(())
    }
}

impl<T: AvTransport> DisplayControl for MacAvServiceBackend<T> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let connector = handle.connector();
        if self.writes_dropped(connector) {
            return Ok(DisplayCapabilities {
                brightness_ddc: false,
                ..DisplayCapabilities::none()
            });
        }
        match self.get_brightness_inner(handle) {
            Ok(_) => Ok(DisplayCapabilities {
                brightness_ddc: true,
                ..DisplayCapabilities::none()
            }),
            Err(error) if error.surfaces_on_probe() => Err(error.into()),
            Err(_) => Ok(DisplayCapabilities::none()),
        }
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        let value = self.get_brightness_inner(handle)?;
        let source = if self.writes_dropped(handle.connector()) {
            BrightnessSource::Gamma
        } else {
            BrightnessSource::Ddc
        };
        Ok(BrightnessState { value, source })
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_brightness_inner(handle, value)
            .map_err(MonitorError::from)
    }

    fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }

    fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }
}

impl<T: AvTransport> DdcStatus for MacAvServiceBackend<T> {
    fn writes_dropped(&self, connector: &str) -> bool {
        self.writes_dropped(connector)
    }
}

pub struct UnsupportedGamma;

impl DisplayControl for UnsupportedGamma {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        Ok(DisplayCapabilities::none())
    }

    fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        Err(MonitorError::unsupported("brightness", GAMMA_REASON))
    }

    fn set_brightness(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("brightness", GAMMA_REASON))
    }

    fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }

    fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }
}

impl GammaStateControl for UnsupportedGamma {
    fn mismatch_count(&self, _handle: &DisplayHandle) -> usize {
        0
    }

    fn warned(&self, _handle: &DisplayHandle) -> bool {
        false
    }

    fn restore(&self, _handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        Ok(RestoreOutcome::NothingToRestore)
    }
}

impl LutProvider for UnsupportedGamma {
    fn capture(&self, _connector: &str) -> Option<crate::monitor::GammaTable> {
        None
    }

    fn write_guarded(
        &self,
        _handle: &DisplayHandle,
        _original: &crate::monitor::GammaTable,
        _last_value: u8,
    ) -> LutRestoreOutcome {
        LutRestoreOutcome::Unavailable
    }
}

#[cfg(target_os = "macos")]
mod iokit {
    use std::ffi::{c_char, c_void};

    use super::{AvError, AvService, AvSymbolResolver, AvTransport, ResolvedSymbols};

    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CF_COMPARE_EQUAL: isize = 0;

    type CreateWithServiceFn = unsafe extern "C" fn(u32, u32) -> *mut c_void;
    type WriteI2cFn = unsafe extern "C" fn(*mut c_void, u32, u32, *const u8, u32) -> i32;
    type ReadI2cFn = unsafe extern "C" fn(*mut c_void, u32, u32, *mut u8, u32) -> i32;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingServices(
            main_port: u32,
            matching: *const c_void,
            existing: *mut u32,
        ) -> i32;
        fn IOIteratorNext(iterator: u32) -> u32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: *const c_void,
            allocator: *const c_void,
            options: u32,
        ) -> *const c_void;
        fn IOObjectRelease(object: u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_string: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFStringCompare(string1: *const c_void, string2: *const c_void, options: u64) -> isize;
        fn CFRelease(value: *const c_void);
    }

    fn cf_string(text: *const c_char) -> *const c_void {
        unsafe { CFStringCreateWithCString(std::ptr::null(), text, K_CF_STRING_ENCODING_UTF8) }
    }

    pub struct DlsymResolver;

    impl AvSymbolResolver for DlsymResolver {
        fn resolve(&self, name: &str) -> Result<*mut c_void, AvError> {
            let c_name = std::ffi::CString::new(name).map_err(|_| AvError::MissingSymbol {
                name: name.to_string(),
            })?;
            let symbol = unsafe { dlsym(RTLD_DEFAULT, c_name.as_ptr()) };
            if symbol.is_null() {
                return Err(AvError::MissingSymbol {
                    name: name.to_string(),
                });
            }
            Ok(symbol)
        }
    }

    pub struct IokitAvTransport {
        symbols: ResolvedSymbols<DlsymResolver>,
    }

    impl IokitAvTransport {
        pub fn new() -> Self {
            Self {
                symbols: ResolvedSymbols::new(DlsymResolver),
            }
        }
    }

    impl Default for IokitAvTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AvTransport for IokitAvTransport {
        fn list_external_services(&self) -> Result<Vec<AvService>, AvError> {
            let create_with_service = self.symbols.get("IOAVServiceCreateWithService")?;
            let create_with_service: CreateWithServiceFn =
                unsafe { std::mem::transmute(create_with_service) };
            let matching = unsafe { IOServiceMatching(c"DCPAVServiceProxy".as_ptr()) };
            if matching.is_null() {
                return Err(AvError::Unsupported {
                    detail: "IOServiceMatching produced no dictionary for DCPAVServiceProxy".into(),
                });
            }
            let mut iterator = 0u32;
            let result = unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) };
            if result != 0 || iterator == 0 {
                return Err(AvError::Unsupported {
                    detail: format!(
                        "IOServiceGetMatchingServices failed with code {result} for \
                         DCPAVServiceProxy"
                    ),
                });
            }
            let location_key = cf_string(c"Location".as_ptr());
            let external = cf_string(c"External".as_ptr());
            let mut services = Vec::new();
            loop {
                let entry = unsafe { IOIteratorNext(iterator) };
                if entry == 0 {
                    break;
                }
                let property = unsafe {
                    IORegistryEntryCreateCFProperty(entry, location_key, std::ptr::null(), 0)
                };
                if !property.is_null() {
                    if unsafe { CFStringCompare(property, external, 0) } == K_CF_COMPARE_EQUAL {
                        let service = unsafe { create_with_service(0, entry) };
                        if !service.is_null() {
                            services.push(AvService::new(service));
                        }
                    }
                    unsafe { CFRelease(property) };
                }
                unsafe { IOObjectRelease(entry) };
            }
            unsafe {
                IOObjectRelease(iterator);
                CFRelease(location_key);
                CFRelease(external);
            }
            Ok(services)
        }

        fn write(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError> {
            let write_i2c = self.symbols.get("IOAVServiceWriteI2C")?;
            let write_i2c: WriteI2cFn = unsafe { std::mem::transmute(write_i2c) };
            let result = unsafe {
                write_i2c(
                    service.raw(),
                    super::I2C_BUS_ID,
                    super::I2C_DATA_ADDRESS,
                    payload.as_ptr(),
                    payload.len() as u32,
                )
            };
            if result != 0 {
                return Err(AvError::Unsupported {
                    detail: format!("IOAVServiceWriteI2C failed with code {result}"),
                });
            }
            Ok(())
        }

        fn read(&self, service: &AvService, buffer: &mut [u8]) -> Result<(), AvError> {
            let read_i2c = self.symbols.get("IOAVServiceReadI2C")?;
            let read_i2c: ReadI2cFn = unsafe { std::mem::transmute(read_i2c) };
            let result = unsafe {
                read_i2c(
                    service.raw(),
                    super::I2C_BUS_ID,
                    super::I2C_DATA_ADDRESS,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                )
            };
            if result != 0 {
                return Err(AvError::Unsupported {
                    detail: format!("IOAVServiceReadI2C failed with code {result}"),
                });
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "macos")]
pub use iokit::IokitAvTransport;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::monitor::backends::i2c_ddc::xor_checksum;
    use crate::monitor::BrightnessSource;

    const HOST_ADDRESS: u8 = 0x51;
    const MONITOR_ADDRESS: u8 = 0x6e;
    const LENGTH_GET: u8 = 0x82;
    const LENGTH_SET: u8 = 0x84;
    const LENGTH_REPLY: u8 = 0x88;
    const OP_GET_VCP: u8 = 0x01;
    const OP_SET_VCP: u8 = 0x03;
    const OP_GET_VCP_REPLY: u8 = 0x02;
    const REPLY_VIRTUAL_HOST: u8 = 0x50;

    #[derive(Clone)]
    struct FakeResolver {
        missing: Option<&'static str>,
    }

    impl AvSymbolResolver for FakeResolver {
        fn resolve(&self, name: &str) -> Result<*mut c_void, AvError> {
            if self.missing == Some(name) {
                Err(AvError::MissingSymbol {
                    name: name.to_string(),
                })
            } else {
                Ok(std::ptr::null_mut())
            }
        }
    }

    struct FakeMonitor {
        current: u16,
        max: u16,
        drop_writes: bool,
        frames_written: Vec<Vec<u8>>,
        pending_reply: Option<Vec<u8>>,
    }

    impl FakeMonitor {
        fn new(current: u16, max: u16) -> Self {
            Self {
                current,
                max,
                drop_writes: false,
                frames_written: Vec::new(),
                pending_reply: None,
            }
        }
    }

    struct FakeAvTransport {
        monitor: Arc<Mutex<FakeMonitor>>,
        symbols: ResolvedSymbols<FakeResolver>,
        services: usize,
    }

    impl FakeAvTransport {
        fn new(monitor: FakeMonitor, services: usize) -> Self {
            Self {
                monitor: Arc::new(Mutex::new(monitor)),
                symbols: ResolvedSymbols::new(FakeResolver { missing: None }),
                services,
            }
        }
    }

    impl AvTransport for FakeAvTransport {
        fn list_external_services(&self) -> Result<Vec<AvService>, AvError> {
            self.symbols.get("IOAVServiceCreateWithService")?;
            Ok((0..self.services)
                .map(|index| AvService::new(index as *mut c_void))
                .collect())
        }

        fn write(&self, _service: &AvService, payload: &[u8]) -> Result<(), AvError> {
            self.symbols.get("IOAVServiceWriteI2C")?;
            let mut monitor = self.monitor.lock().unwrap();
            monitor.frames_written.push(payload.to_vec());
            if payload.len() < 2 || !matches!(payload[0], LENGTH_GET | LENGTH_SET) {
                return Err(AvError::Protocol {
                    detail: "request carries an unknown DDC/CI length byte".into(),
                });
            }
            let mut frame = Vec::with_capacity(payload.len() + 1);
            frame.push(HOST_ADDRESS);
            frame.extend_from_slice(payload);
            if xor_checksum(MONITOR_ADDRESS, &frame[..frame.len() - 1]) != frame[frame.len() - 1] {
                return Err(AvError::Protocol {
                    detail: "request checksum mismatch".into(),
                });
            }
            match frame[2] {
                OP_SET_VCP => {
                    let value = u16::from_be_bytes([frame[4], frame[5]]);
                    if !monitor.drop_writes {
                        monitor.current = value;
                    }
                }
                OP_GET_VCP => {
                    let mut reply = [0u8; REPLY_LEN];
                    reply[0] = MONITOR_ADDRESS;
                    reply[1] = LENGTH_REPLY;
                    reply[2] = OP_GET_VCP_REPLY;
                    reply[4] = frame[3];
                    reply[6] = (monitor.max >> 8) as u8;
                    reply[7] = monitor.max as u8;
                    reply[8] = (monitor.current >> 8) as u8;
                    reply[9] = monitor.current as u8;
                    reply[10] = xor_checksum(REPLY_VIRTUAL_HOST, &reply[..10]);
                    monitor.pending_reply = Some(reply.to_vec());
                }
                _ => {
                    return Err(AvError::Protocol {
                        detail: "unknown DDC/CI opcode".into(),
                    })
                }
            }
            Ok(())
        }

        fn read(&self, _service: &AvService, buffer: &mut [u8]) -> Result<(), AvError> {
            self.symbols.get("IOAVServiceReadI2C")?;
            let mut monitor = self.monitor.lock().unwrap();
            let Some(pending) = monitor.pending_reply.take() else {
                return Ok(());
            };
            let count = pending.len().min(buffer.len());
            buffer[..count].copy_from_slice(&pending[..count]);
            Ok(())
        }
    }

    fn handle(connector: &str) -> DisplayHandle {
        DisplayHandle::new(format!("id-{connector}"), connector.into(), None, false)
    }

    fn displays(connectors: &[&str]) -> Vec<DisplayHandle> {
        connectors
            .iter()
            .map(|connector| handle(connector))
            .collect()
    }

    fn backend(
        monitor: FakeMonitor,
        services: usize,
        connectors: &[&str],
    ) -> MacAvServiceBackend<FakeAvTransport> {
        let owned = displays(connectors);
        let mut backend = MacAvServiceBackend::with_timing(
            FakeAvTransport::new(monitor, services),
            Duration::ZERO,
            Duration::ZERO,
        );
        backend.enumerate_displays = Box::new(move || Ok(owned.clone()));
        backend
    }

    #[test]
    fn get_request_payload_matches_the_canonical_ddc_ci_bytes() {
        let backend = backend(FakeMonitor::new(0, 100), 1, &["card0-DP-1"]);
        backend.get_brightness(&handle("card0-DP-1")).unwrap();
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(
            monitor.frames_written[0].as_slice(),
            &[0x82, 0x01, 0x10, 0xac]
        );
    }

    #[test]
    fn set_request_payload_matches_the_canonical_ddc_ci_bytes() {
        let backend = backend(FakeMonitor::new(0, 100), 1, &["card0-DP-1"]);
        backend.set_brightness(&handle("card0-DP-1"), 50).unwrap();
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(
            monitor.frames_written[1].as_slice(),
            &[0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]
        );
    }

    #[test]
    fn get_reads_brightness_over_the_paired_service() {
        let backend = backend(FakeMonitor::new(200, 1000), 1, &["card0-DP-1"]);
        let state = backend.get_brightness(&handle("card0-DP-1")).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Ddc);
    }

    #[test]
    fn builtin_connectors_are_skipped_when_pairing() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            1,
            &["card0-DP-1", "AppleCLCD2-builtin"],
        );
        let state = backend.get_brightness(&handle("card0-DP-1")).unwrap();
        assert_eq!(state.value, 20);
    }

    #[test]
    fn count_mismatch_names_both_counts() {
        let backend = backend(FakeMonitor::new(200, 1000), 2, &["card0-DP-1"]);
        let error = backend.get_brightness(&handle("card0-DP-1")).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("1 external"), "{reason}");
                assert!(reason.contains("2 DCPAVServiceProxy"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
        let caps = backend.probe(&handle("card0-DP-1")).unwrap_err();
        assert!(matches!(
            caps,
            MonitorError::Unsupported {
                capability: "brightness",
                ..
            }
        ));
    }

    #[test]
    fn missing_private_symbol_surfaces_a_typed_error_with_the_symbol_name() {
        let monitor = FakeMonitor::new(200, 1000);
        let mut transport = FakeAvTransport::new(monitor, 1);
        transport.symbols.resolver.missing = Some("IOAVServiceCreateWithService");
        let owned = displays(&["card0-DP-1"]);
        let mut backend =
            MacAvServiceBackend::with_timing(transport, Duration::ZERO, Duration::ZERO);
        backend.enumerate_displays = Box::new(move || Ok(owned.clone()));
        let error = backend.get_brightness(&handle("card0-DP-1")).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("IOAVServiceCreateWithService"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn dropped_writes_downgrade_the_source_and_disable_the_capability() {
        let monitor = FakeMonitor {
            drop_writes: true,
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(monitor, 1, &["card0-DP-1"]);
        backend.set_brightness(&handle("card0-DP-1"), 50).unwrap();
        {
            let monitor = backend.transport.monitor.lock().unwrap();
            assert_eq!(
                monitor.frames_written.len(),
                5,
                "pre-read, two set attempts, two verify read-backs"
            );
        }
        let state = backend.get_brightness(&handle("card0-DP-1")).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Gamma);
        let caps = backend.probe(&handle("card0-DP-1")).unwrap();
        assert!(!caps.brightness_ddc);
        let error = backend
            .set_brightness(&handle("card0-DP-1"), 60)
            .unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("writes were dropped"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}
