use std::collections::HashSet;
use std::ffi::c_void;
use std::fmt;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use qol_windowing::DisplayEnumerator;

use crate::monitor::backends::i2c_ddc::{
    parse_get_vcp_reply, percent_from_raw, xor_checksum, I2cError, REPLY_LEN,
};
use crate::monitor::policy::DdcStatus;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, HdrState, MonitorError, GAMMA_REASON, HDR_REASON, MODES_REASON,
};

const FEATURE_BRIGHTNESS: u8 = 0x10;
const HOST_ADDRESS: u8 = 0x51;
const MONITOR_ADDRESS: u8 = 0x6e;
const LENGTH_GET: u8 = 0x82;
const LENGTH_SET: u8 = 0x84;
const OP_GET_VCP: u8 = 0x01;
const OP_SET_VCP: u8 = 0x03;
const WRITE_ATTEMPTS: usize = 5;
const WRITE_DELAY: Duration = Duration::from_millis(10);
const READ_DELAY: Duration = Duration::from_millis(50);
const RETRY_DELAY: Duration = Duration::from_millis(20);

pub fn get_vcp_payload(feature: u8) -> [u8; 4] {
    let mut payload = [LENGTH_GET, OP_GET_VCP, feature, 0];
    payload[3] = xor_checksum(MONITOR_ADDRESS, &payload[..3]);
    payload
}

pub fn set_vcp_payload(feature: u8, value: u16) -> [u8; 6] {
    let mut payload = [
        LENGTH_SET,
        OP_SET_VCP,
        feature,
        (value >> 8) as u8,
        value as u8,
        0,
    ];
    payload[5] = xor_checksum(MONITOR_ADDRESS ^ HOST_ADDRESS, &payload[..5]);
    payload
}

#[cfg(any(target_os = "macos", test))]
pub fn pnp_to_vendor_id(pnp: &str) -> u32 {
    let bytes = pnp.as_bytes();
    if bytes.len() < 3 {
        return 0;
    }
    let letter = |byte: u8| u32::from(byte.wrapping_sub(b'A') + 1);
    (letter(bytes[0]) << 10) | (letter(bytes[1]) << 5) | letter(bytes[2])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayIdentity {
    pub vendor: u32,
    pub model: u32,
    pub serial: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportClass {
    ConverterRouted,
    DirectDp,
}

#[derive(Debug)]
pub enum AvError {
    MissingSymbol {
        name: String,
    },
    Unmatched {
        vendor: u32,
        model: u32,
        serial: u32,
    },
    ConverterRoutedWritesDisabled,
    Protocol {
        detail: String,
    },
    Unsupported {
        detail: String,
    },
}

impl AvError {
    fn surfaces_on_probe(&self) -> bool {
        matches!(self, Self::MissingSymbol { .. })
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
            Self::Unmatched {
                vendor,
                model,
                serial,
            } => write!(
                f,
                "no DCPAVServiceProxy service carries the identity (vendor 0x{vendor:04x}, \
                 model 0x{model:04x}, serial 0x{serial:04x})"
            ),
            Self::ConverterRoutedWritesDisabled => write!(
                f,
                "converter-routed HDMI DDC writes are disabled because they can crash the \
                 display; the gamma fallback owns brightness there"
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

impl From<AvError> for MonitorError {
    fn from(error: AvError) -> Self {
        match error {
            AvError::MissingSymbol { name } => Self::unsupported(
                "brightness",
                format!("{name} is not exported by this macOS; IOAVService DDC is unavailable"),
            ),
            AvError::Unmatched {
                vendor,
                model,
                serial,
            } => Self::unsupported(
                "brightness",
                format!(
                    "no DCPAVServiceProxy service carries the identity (vendor 0x{vendor:04x}, \
                     model 0x{model:04x}, serial 0x{serial:04x})"
                ),
            ),
            AvError::ConverterRoutedWritesDisabled => Self::refused(
                "brightness",
                "converter-routed HDMI DDC writes are disabled because they can crash the \
                 display; the gamma fallback owns brightness there",
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
    pub(crate) fn new(raw: *mut c_void) -> Self {
        Self { raw }
    }

    pub(crate) fn raw(&self) -> *mut c_void {
        self.raw
    }
}

#[derive(Debug)]
pub struct AvServiceInfo {
    pub service: AvService,
    pub identity: DisplayIdentity,
    pub transport_class: TransportClass,
}

pub trait AvSymbolResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Result<*mut c_void, AvError>;
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct ResolvedSymbols<R: AvSymbolResolver> {
    resolver: R,
    cache: Mutex<Vec<(&'static str, usize)>>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl<R: AvSymbolResolver> ResolvedSymbols<R> {
    pub(crate) fn new(resolver: R) -> Self {
        Self {
            resolver,
            cache: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn get(&self, name: &'static str) -> Result<usize, AvError> {
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
    fn list_external_services(&self) -> Result<Vec<AvServiceInfo>, AvError>;
    fn write(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError>;
    fn read(&self, service: &AvService, buffer: &mut [u8]) -> Result<(), AvError>;
}

pub type IdentityResolver =
    Box<dyn Fn(&DisplayHandle) -> Result<Option<DisplayIdentity>, AvError> + Send + Sync>;

pub struct MacAvServiceBackend<T: AvTransport> {
    transport: T,
    write_delay: Duration,
    read_delay: Duration,
    retry_delay: Duration,
    identity_for: IdentityResolver,
    dropped_writes: Mutex<HashSet<String>>,
}

impl<T: AvTransport> MacAvServiceBackend<T> {
    pub fn new(transport: T, identity_for: IdentityResolver) -> Self {
        let mut backend = Self::with_timing(transport, WRITE_DELAY, READ_DELAY, RETRY_DELAY);
        backend.identity_for = identity_for;
        backend
    }

    pub fn with_timing(
        transport: T,
        write_delay: Duration,
        read_delay: Duration,
        retry_delay: Duration,
    ) -> Self {
        Self {
            transport,
            write_delay,
            read_delay,
            retry_delay,
            identity_for: Box::new(|_| Ok(None)),
            dropped_writes: Mutex::new(HashSet::new()),
        }
    }

    fn writes_dropped(&self, connector: &str) -> bool {
        self.dropped_writes.lock().unwrap().contains(connector)
    }

    fn service_for<'a>(
        &self,
        handle: &DisplayHandle,
        services: &'a [AvServiceInfo],
    ) -> Result<&'a AvServiceInfo, AvError> {
        let Some(identity) = (self.identity_for)(handle)? else {
            return Err(AvError::Unsupported {
                detail: format!("no CG display id parses from {}", handle.connector()),
            });
        };
        services
            .iter()
            .find(|info| info.identity == identity)
            .ok_or(AvError::Unmatched {
                vendor: identity.vendor,
                model: identity.model,
                serial: identity.serial,
            })
    }

    fn write_payload(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError> {
        thread::sleep(self.write_delay);
        self.transport.write(service, payload)?;
        thread::sleep(self.write_delay);
        self.transport.write(service, payload)
    }

    fn read_current_max(&self, service: &AvService) -> Result<(u16, u16), AvError> {
        let payload = get_vcp_payload(FEATURE_BRIGHTNESS);
        self.write_payload(service, &payload)?;
        thread::sleep(self.read_delay);
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
        let (current, max) = self.read_current_max(&service.service)?;
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
        if service.transport_class == TransportClass::ConverterRouted {
            return Err(AvError::ConverterRoutedWritesDisabled);
        }
        let (_, max) = self.read_current_max(&service.service)?;
        if max == 0 {
            return Err(AvError::Protocol {
                detail: format!("{connector} reports a maximum brightness of 0"),
            });
        }
        let target = (u32::from(value) * u32::from(max) / 100) as u16;
        let mut verified = false;
        for attempt in 0..WRITE_ATTEMPTS {
            let payload = set_vcp_payload(FEATURE_BRIGHTNESS, target);
            self.write_payload(&service.service, &payload)?;
            if self.read_back_matches(&service.service, value)? {
                verified = true;
                break;
            }
            if attempt + 1 < WRITE_ATTEMPTS {
                thread::sleep(self.retry_delay);
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
        let services = match self.transport.list_external_services() {
            Ok(services) => services,
            Err(error) if error.surfaces_on_probe() => return Err(error.into()),
            Err(_) => return Ok(DisplayCapabilities::none()),
        };
        let service = match self.service_for(handle, &services) {
            Ok(service) => service,
            Err(error) if error.surfaces_on_probe() => return Err(error.into()),
            Err(_) => return Ok(DisplayCapabilities::none()),
        };
        if service.transport_class == TransportClass::ConverterRouted {
            return Ok(DisplayCapabilities::none());
        }
        match self.read_current_max(&service.service) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::monitor::BrightnessSource;

    const LENGTH_REPLY: u8 = 0x88;
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
        ignore_sets_until: usize,
        set_writes: usize,
        frames_written: Vec<Vec<u8>>,
        pending_reply: Option<Vec<u8>>,
        read_after_writes: Vec<usize>,
    }

    impl FakeMonitor {
        fn new(current: u16, max: u16) -> Self {
            Self {
                current,
                max,
                drop_writes: false,
                ignore_sets_until: 0,
                set_writes: 0,
                frames_written: Vec::new(),
                pending_reply: None,
                read_after_writes: Vec::new(),
            }
        }
    }

    #[derive(Clone, Copy)]
    struct ServiceSpec {
        identity: DisplayIdentity,
        transport_class: TransportClass,
    }

    struct FakeAvTransport {
        monitors: Vec<Arc<Mutex<FakeMonitor>>>,
        symbols: ResolvedSymbols<FakeResolver>,
        specs: Vec<ServiceSpec>,
    }

    impl FakeAvTransport {
        fn new(specs: Vec<ServiceSpec>, monitors: Vec<FakeMonitor>) -> Self {
            Self {
                monitors: monitors
                    .into_iter()
                    .map(|monitor| Arc::new(Mutex::new(monitor)))
                    .collect(),
                symbols: ResolvedSymbols::new(FakeResolver { missing: None }),
                specs,
            }
        }

        fn monitor(&self, service: &AvService) -> &Arc<Mutex<FakeMonitor>> {
            &self.monitors[service.raw() as usize]
        }
    }

    impl AvTransport for FakeAvTransport {
        fn list_external_services(&self) -> Result<Vec<AvServiceInfo>, AvError> {
            self.symbols.get("IOAVServiceCreateWithService")?;
            Ok(self
                .specs
                .iter()
                .enumerate()
                .map(|(index, spec)| AvServiceInfo {
                    service: AvService::new(index as *mut c_void),
                    identity: spec.identity,
                    transport_class: spec.transport_class,
                })
                .collect())
        }

        fn write(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError> {
            self.symbols.get("IOAVServiceWriteI2C")?;
            let mut monitor = self.monitor(service).lock().unwrap();
            monitor.frames_written.push(payload.to_vec());
            if payload.len() < 4 || !matches!(payload[0], LENGTH_GET | LENGTH_SET) {
                return Err(AvError::Protocol {
                    detail: "request carries an unknown DDC/CI length byte".into(),
                });
            }
            let seed = match payload[0] {
                LENGTH_GET => MONITOR_ADDRESS,
                LENGTH_SET => MONITOR_ADDRESS ^ HOST_ADDRESS,
                _ => unreachable!(),
            };
            if xor_checksum(seed, &payload[..payload.len() - 1]) != payload[payload.len() - 1] {
                return Err(AvError::Protocol {
                    detail: "request checksum mismatch".into(),
                });
            }
            match payload[1] {
                OP_SET_VCP => {
                    let value = u16::from_be_bytes([payload[3], payload[4]]);
                    if !monitor.drop_writes && monitor.set_writes >= monitor.ignore_sets_until {
                        monitor.current = value;
                    }
                    monitor.set_writes += 1;
                }
                OP_GET_VCP => {
                    let mut reply = [0u8; REPLY_LEN];
                    reply[0] = MONITOR_ADDRESS;
                    reply[1] = LENGTH_REPLY;
                    reply[2] = OP_GET_VCP_REPLY;
                    reply[4] = payload[2];
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

        fn read(&self, service: &AvService, buffer: &mut [u8]) -> Result<(), AvError> {
            self.symbols.get("IOAVServiceReadI2C")?;
            let mut monitor = self.monitor(service).lock().unwrap();
            let writes_so_far = monitor.frames_written.len();
            monitor.read_after_writes.push(writes_so_far);
            let Some(pending) = monitor.pending_reply.take() else {
                return Ok(());
            };
            let count = pending.len().min(buffer.len());
            buffer[..count].copy_from_slice(&pending[..count]);
            Ok(())
        }
    }

    fn handle(connector: &str) -> DisplayHandle {
        DisplayHandle::new(format!("mac-{connector}"), connector.into(), None, false)
    }

    fn identity(vendor: u32, model: u32, serial: u32) -> DisplayIdentity {
        DisplayIdentity {
            vendor,
            model,
            serial,
        }
    }

    fn direct(identity: DisplayIdentity) -> ServiceSpec {
        ServiceSpec {
            identity,
            transport_class: TransportClass::DirectDp,
        }
    }

    fn backend(
        monitor: FakeMonitor,
        services: &[ServiceSpec],
        identities: &[(&str, DisplayIdentity)],
    ) -> MacAvServiceBackend<FakeAvTransport> {
        let mut backend = MacAvServiceBackend::with_timing(
            FakeAvTransport::new(services.to_vec(), vec![monitor]),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        let owned: Vec<(String, DisplayIdentity)> = identities
            .iter()
            .map(|(connector, identity)| (connector.to_string(), *identity))
            .collect();
        backend.identity_for = Box::new(move |handle| {
            Ok(owned
                .iter()
                .find(|(connector, _)| connector.as_str() == handle.connector())
                .map(|(_, identity)| *identity))
        });
        backend
    }

    #[test]
    fn get_payload_matches_the_canonical_ddc_ci_bytes() {
        assert_eq!(
            get_vcp_payload(FEATURE_BRIGHTNESS),
            [0x82, 0x01, 0x10, 0xfd]
        );
        let backend = backend(
            FakeMonitor::new(0, 100),
            &[direct(identity(1, 2, 3))],
            &[("cg-1", identity(1, 2, 3))],
        );
        backend.get_brightness(&handle("cg-1")).unwrap();
        let monitor = backend.transport.monitors[0].lock().unwrap();
        assert_eq!(
            monitor.frames_written[0].as_slice(),
            &[0x82, 0x01, 0x10, 0xfd]
        );
    }

    #[test]
    fn set_payload_matches_the_canonical_ddc_ci_bytes() {
        assert_eq!(
            set_vcp_payload(FEATURE_BRIGHTNESS, 0x0032),
            [0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]
        );
        let backend = backend(
            FakeMonitor::new(0, 100),
            &[direct(identity(1, 2, 3))],
            &[("cg-1", identity(1, 2, 3))],
        );
        backend.set_brightness(&handle("cg-1"), 50).unwrap();
        let monitor = backend.transport.monitors[0].lock().unwrap();
        assert_eq!(
            monitor.frames_written[2].as_slice(),
            &[0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]
        );
        assert_eq!(
            monitor.frames_written[3].as_slice(),
            &[0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]
        );
    }

    #[test]
    fn every_logical_write_is_issued_twice_and_the_read_follows() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            &[direct(identity(1, 2, 3))],
            &[("cg-1", identity(1, 2, 3))],
        );
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Ddc);
        let monitor = backend.transport.monitors[0].lock().unwrap();
        assert_eq!(
            monitor.frames_written.len(),
            2,
            "one logical get write, issued twice"
        );
        assert_eq!(monitor.frames_written[0], monitor.frames_written[1]);
        assert_eq!(
            monitor.read_after_writes,
            vec![2],
            "the read happens after both write cycles"
        );
    }

    #[test]
    fn retries_stop_on_the_first_valid_reply() {
        let monitor = FakeMonitor {
            ignore_sets_until: 2,
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(
            monitor,
            &[direct(identity(1, 2, 3))],
            &[("cg-1", identity(1, 2, 3))],
        );
        backend.set_brightness(&handle("cg-1"), 50).unwrap();
        let monitor = backend.transport.monitors[0].lock().unwrap();
        assert_eq!(
            monitor.frames_written.len(),
            10,
            "pre-read, one ignored set attempt, one verified set attempt"
        );
        assert_eq!(
            monitor.read_after_writes,
            vec![2, 6, 10],
            "reads follow their write cycles"
        );
        assert_eq!(monitor.current, 500, "the second attempt applied");
        for pair in monitor.frames_written.chunks(2) {
            assert_eq!(pair[0], pair[1], "every logical write is issued twice");
        }
    }

    #[test]
    fn services_pair_to_displays_by_identity_regardless_of_order() {
        let specs = vec![
            direct(identity(0x10ac, 0x4321, 0x8765)),
            direct(identity(0x4c2d, 0x1234, 0x5678)),
        ];
        let monitors = vec![FakeMonitor::new(100, 1000), FakeMonitor::new(250, 1000)];
        let mut backend = MacAvServiceBackend::with_timing(
            FakeAvTransport::new(specs, monitors),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        let owned: Vec<(String, DisplayIdentity)> = vec![
            ("cg-1".to_string(), identity(0x4c2d, 0x1234, 0x5678)),
            ("cg-2".to_string(), identity(0x10ac, 0x4321, 0x8765)),
        ];
        backend.identity_for = Box::new(move |handle| {
            Ok(owned
                .iter()
                .find(|(connector, _)| connector.as_str() == handle.connector())
                .map(|(_, identity)| *identity))
        });
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(
            state.value, 25,
            "display A reads through the service listed second"
        );
        let state = backend.get_brightness(&handle("cg-2")).unwrap();
        assert_eq!(
            state.value, 10,
            "display B reads through the service listed first"
        );
    }

    #[test]
    fn unmatched_display_errors_with_its_identity_tuple() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            &[direct(identity(0x10ac, 0x4321, 0x8765))],
            &[("cg-1", identity(0x4c2d, 0x1234, 0x5678))],
        );
        let error = backend.get_brightness(&handle("cg-1")).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("vendor 0x4c2d"), "{reason}");
                assert!(reason.contains("model 0x1234"), "{reason}");
                assert!(reason.contains("serial 0x5678"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn builtin_displays_error_out_but_do_not_affect_external_pairing() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            &[direct(identity(0x4c2d, 0x1234, 0x5678))],
            &[
                ("cg-1", identity(0x4c2d, 0x1234, 0x5678)),
                ("cg-2-builtin", identity(0x06af, 0x9e40, 0x0001)),
            ],
        );
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(state.value, 20);
        assert!(matches!(
            backend.get_brightness(&handle("cg-2-builtin")),
            Err(MonitorError::Unsupported {
                capability: "brightness",
                ..
            })
        ));
    }

    #[test]
    fn converter_routed_services_refuse_writes_and_report_no_ddc() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            &[ServiceSpec {
                identity: identity(0x4c2d, 0x1234, 0x5678),
                transport_class: TransportClass::ConverterRouted,
            }],
            &[("cg-1", identity(0x4c2d, 0x1234, 0x5678))],
        );
        match backend.set_brightness(&handle("cg-1"), 50).unwrap_err() {
            MonitorError::Refused { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("converter-routed"), "{reason}");
                assert!(reason.contains("crash the display"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        let caps = backend.probe(&handle("cg-1")).unwrap();
        assert!(!caps.brightness_ddc);
        let monitor = backend.transport.monitors[0].lock().unwrap();
        assert_eq!(
            monitor.frames_written.len(),
            0,
            "a refused write never touches the transport"
        );
    }

    #[test]
    fn direct_dp_services_allow_writes() {
        let backend = backend(
            FakeMonitor::new(200, 1000),
            &[direct(identity(0x4c2d, 0x1234, 0x5678))],
            &[("cg-1", identity(0x4c2d, 0x1234, 0x5678))],
        );
        backend.set_brightness(&handle("cg-1"), 50).unwrap();
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(state.value, 50);
        assert_eq!(state.source, BrightnessSource::Ddc);
        let caps = backend.probe(&handle("cg-1")).unwrap();
        assert!(caps.brightness_ddc);
    }

    #[test]
    fn dropped_writes_downgrade_the_source_and_disable_the_capability() {
        let monitor = FakeMonitor {
            drop_writes: true,
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(
            monitor,
            &[direct(identity(0x4c2d, 0x1234, 0x5678))],
            &[("cg-1", identity(0x4c2d, 0x1234, 0x5678))],
        );
        backend.set_brightness(&handle("cg-1"), 50).unwrap();
        {
            let monitor = backend.transport.monitors[0].lock().unwrap();
            assert_eq!(
                monitor.frames_written.len(),
                22,
                "pre-read plus five attempts, every logical write twice"
            );
            assert_eq!(monitor.read_after_writes, vec![2, 6, 10, 14, 18, 22]);
        }
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Gamma);
        let caps = backend.probe(&handle("cg-1")).unwrap();
        assert!(!caps.brightness_ddc);
        let error = backend.set_brightness(&handle("cg-1"), 60).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("writes were dropped"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn missing_private_symbol_surfaces_a_typed_error_with_the_symbol_name() {
        let monitor = FakeMonitor::new(200, 1000);
        let mut transport = FakeAvTransport::new(
            vec![direct(identity(0x4c2d, 0x1234, 0x5678))],
            vec![monitor],
        );
        transport.symbols.resolver.missing = Some("IOAVServiceCreateWithService");
        let owned: Vec<(String, DisplayIdentity)> =
            vec![("cg-1".to_string(), identity(0x4c2d, 0x1234, 0x5678))];
        let mut backend = MacAvServiceBackend::with_timing(
            transport,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        backend.identity_for = Box::new(move |handle| {
            Ok(owned
                .iter()
                .find(|(connector, _)| connector.as_str() == handle.connector())
                .map(|(_, identity)| *identity))
        });
        let error = backend.get_brightness(&handle("cg-1")).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("IOAVServiceCreateWithService"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn pnp_vendor_conversion_matches_the_edid_formula() {
        assert_eq!(pnp_to_vendor_id("SAM"), 0x4c2d);
        assert_eq!(pnp_to_vendor_id("DEL"), 0x10ac);
        assert_eq!(pnp_to_vendor_id("GSM"), 0x1e6d);
    }
}
