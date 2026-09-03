use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use qol_windowing::DisplayEnumerator;

use crate::monitor::night::Tint;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, GammaStateControl, HdrState, MonitorError, RestoreOutcome, HDR_REASON,
    MODES_REASON,
};
use crate::session::{LutProvider, LutRestoreOutcome};

pub const MISMATCH_WARN_AT: usize = 3;

#[derive(Debug, Clone)]
pub enum GammaError {
    Unsupported { detail: String },
    Refused { reason: String },
}

impl GammaError {
    fn into_monitor(self, capability: &'static str) -> MonitorError {
        match self {
            Self::Unsupported { detail } => MonitorError::unsupported(capability, detail),
            Self::Refused { reason } => MonitorError::refused(capability, reason),
        }
    }
}

impl fmt::Display for GammaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { detail } => write!(f, "{detail}"),
            Self::Refused { reason } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for GammaError {}

impl From<GammaError> for MonitorError {
    fn from(error: GammaError) -> Self {
        error.into_monitor("gamma")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GammaTable {
    pub red: Vec<u16>,
    pub green: Vec<u16>,
    pub blue: Vec<u16>,
}

impl GammaTable {
    pub fn size(&self) -> usize {
        self.red.len()
    }

    pub fn peak(&self) -> u16 {
        self.red
            .iter()
            .chain(&self.green)
            .chain(&self.blue)
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub fn checksum(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for word in self.red.iter().chain(&self.green).chain(&self.blue) {
            for byte in word.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        hash
    }

    pub fn dimmed(&self, percent: u8) -> Self {
        let factor = u32::from(percent.min(100));
        let dim = |entry: u16| (u32::from(entry) * factor / 100) as u16;
        GammaTable {
            red: self.red.iter().map(|entry| dim(*entry)).collect(),
            green: self.green.iter().map(|entry| dim(*entry)).collect(),
            blue: self.blue.iter().map(|entry| dim(*entry)).collect(),
        }
    }

    pub fn tinted(&self, tint: Tint) -> Self {
        let scale =
            |entry: u16, channel: u16| (u32::from(entry) * u32::from(channel) / 1000) as u16;
        GammaTable {
            red: self
                .red
                .iter()
                .map(|entry| scale(*entry, tint.red))
                .collect(),
            green: self
                .green
                .iter()
                .map(|entry| scale(*entry, tint.green))
                .collect(),
            blue: self
                .blue
                .iter()
                .map(|entry| scale(*entry, tint.blue))
                .collect(),
        }
    }
}

pub trait GammaTransport: Send + Sync {
    type Bus: GammaBus;
    fn open(&self) -> Result<Self::Bus, GammaError>;
}

pub trait GammaBus {
    fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError>;
    fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError>;
    fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError>;
    fn hdr_active(&mut self, crtc: u32) -> Result<bool, GammaError>;
}

#[derive(Default)]
struct GammaSession {
    original: Option<GammaTable>,
    written_checksum: Option<u64>,
    written_value: Option<u8>,
    tint: Tint,
    mismatches: usize,
    warned: bool,
}

pub struct GammaBackend<T: GammaTransport> {
    transport: T,
    sessions: Mutex<HashMap<String, GammaSession>>,
}

impl<T: GammaTransport> GammaBackend<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self) -> std::sync::MutexGuard<'_, HashMap<String, GammaSession>> {
        self.sessions.lock().unwrap()
    }

    fn get_inner(&self, handle: &DisplayHandle) -> Result<u8, GammaError> {
        let mut bus = self.transport.open()?;
        let Some(crtc) = bus.crtc_for_connector(handle.connector())? else {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "no X11 output matches {}; gamma needs a RandR output for this connector",
                    handle.connector()
                ),
            });
        };
        let current = bus.read_gamma(crtc)?;
        if current.size() < 2 {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "the gamma ramp on this output is inert (size {})",
                    current.size()
                ),
            });
        }
        let session = self.session();
        match session.get(handle.id()) {
            Some(entry) => match (entry.written_value, entry.written_checksum) {
                (Some(value), Some(checksum)) if current.checksum() == checksum => Ok(value),
                _ => match &entry.original {
                    Some(original) => Ok(peak_percent(&current, original)),
                    None => Ok(100),
                },
            },
            None => Ok(100),
        }
    }

    fn set_inner(
        &self,
        handle: &DisplayHandle,
        value: Option<u8>,
        tint: Option<Tint>,
    ) -> Result<(), GammaError> {
        let mut bus = self.transport.open()?;
        let Some(crtc) = bus.crtc_for_connector(handle.connector())? else {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "no X11 output matches {}; gamma needs a RandR output for this connector",
                    handle.connector()
                ),
            });
        };
        if matches!(bus.hdr_active(crtc), Ok(true)) {
            return Err(GammaError::Refused {
                reason: "HDR is active; gamma LUT writes are no-ops on HDR displays".into(),
            });
        }
        let current = bus.read_gamma(crtc)?;
        if current.size() < 2 {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "the gamma ramp on this output is inert (size {})",
                    current.size()
                ),
            });
        }
        let mut session = self.session();
        let entry = session.entry(handle.id().to_string()).or_default();
        let original = entry.original.get_or_insert_with(|| current.clone());
        let value = value.unwrap_or_else(|| entry.written_value.unwrap_or(100));
        let tint = tint.unwrap_or(entry.tint);
        let target = original.dimmed(value).tinted(tint);
        let mut verified = false;
        for _ in 0..=1 {
            bus.write_gamma(crtc, &target)?;
            let back = bus.read_gamma(crtc)?;
            if back.size() == target.size() && back.checksum() == target.checksum() {
                verified = true;
                break;
            }
            if back.size() != target.size() {
                break;
            }
        }
        if verified {
            entry.written_checksum = Some(target.checksum());
            entry.written_value = Some(value);
            entry.tint = tint;
        } else {
            entry.mismatches += 1;
            if entry.mismatches >= MISMATCH_WARN_AT {
                entry.warned = true;
            }
        }
        Ok(())
    }

    fn restore_lut(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) -> Result<RestoreOutcome, GammaError> {
        let guard = {
            let session = self.session();
            match session.get(handle.id()) {
                Some(entry) => entry
                    .written_checksum
                    .unwrap_or_else(|| original.dimmed(last_value).tinted(last_tint).checksum()),
                None => original.dimmed(last_value).tinted(last_tint).checksum(),
            }
        };
        let mut bus = self.transport.open()?;
        let Some(crtc) = bus.crtc_for_connector(handle.connector())? else {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "no X11 output matches {}; gamma needs a RandR output for this connector",
                    handle.connector()
                ),
            });
        };
        let current = bus.read_gamma(crtc)?;
        if current.checksum() != guard {
            return Ok(RestoreOutcome::ForeignLutPreserved);
        }
        bus.write_gamma(crtc, original)?;
        let back = bus.read_gamma(crtc)?;
        if back.size() != original.size() || back.checksum() != original.checksum() {
            let mut session = self.session();
            let entry = session.entry(handle.id().to_string()).or_default();
            entry.mismatches += 1;
            if entry.mismatches >= MISMATCH_WARN_AT {
                entry.warned = true;
            }
            return Err(GammaError::Unsupported {
                detail:
                    "the restore write did not verify; a co-owner changed the LUT during restore"
                        .into(),
            });
        }
        let mut session = self.session();
        if let Some(entry) = session.get_mut(handle.id()) {
            entry.original = None;
            entry.written_checksum = None;
            entry.written_value = None;
            entry.tint = Tint::NEUTRAL;
        }
        Ok(RestoreOutcome::Restored)
    }

    fn restore_inner(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, GammaError> {
        let session = self.session();
        let Some(entry) = session.get(handle.id()) else {
            return Ok(RestoreOutcome::NothingToRestore);
        };
        let (Some(original), Some(_)) = (&entry.original, entry.written_checksum) else {
            return Ok(RestoreOutcome::NothingToRestore);
        };
        let last_value = entry.written_value.unwrap_or(100);
        let last_tint = entry.tint;
        let original = original.clone();
        drop(session);
        self.restore_lut(handle, &original, last_value, last_tint)
    }
}

impl<T: GammaTransport> DisplayControl for GammaBackend<T> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let mut bus = self
            .transport
            .open()
            .map_err(|error| error.into_monitor("gamma"))?;
        let Some(crtc) = bus.crtc_for_connector(handle.connector())? else {
            return Ok(DisplayCapabilities::none());
        };
        let size = bus.read_gamma(crtc)?.size();
        Ok(DisplayCapabilities {
            brightness_gamma: size >= 2,
            ..DisplayCapabilities::none()
        })
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        let value = self
            .get_inner(handle)
            .map_err(|error| error.into_monitor("brightness"))?;
        Ok(BrightnessState {
            value,
            source: BrightnessSource::Gamma,
        })
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_inner(handle, Some(value), None)
            .map_err(|error| error.into_monitor("brightness"))
    }

    fn set_tint(&self, handle: &DisplayHandle, tint: Tint) -> Result<(), MonitorError> {
        self.set_inner(handle, None, Some(tint))
            .map_err(|error| error.into_monitor("tint"))
    }

    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        let value = self
            .get_inner(handle)
            .map_err(|error| error.into_monitor("gamma"))?;
        Ok(GammaState { value })
    }

    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_inner(handle, Some(value), None)
            .map_err(|error| error.into_monitor("gamma"))
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

impl<T: GammaTransport> GammaStateControl for GammaBackend<T> {
    fn mismatch_count(&self, handle: &DisplayHandle) -> usize {
        self.session()
            .get(handle.id())
            .map(|entry| entry.mismatches)
            .unwrap_or(0)
    }

    fn warned(&self, handle: &DisplayHandle) -> bool {
        self.session()
            .get(handle.id())
            .map(|entry| entry.warned)
            .unwrap_or(false)
    }

    fn restore(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        self.restore_inner(handle)
            .map_err(|error| error.into_monitor("gamma"))
    }
}

impl<T: GammaTransport> LutProvider for GammaBackend<T> {
    fn capture(&self, connector: &str) -> Option<GammaTable> {
        let mut bus = self.transport.open().ok()?;
        let crtc = bus.crtc_for_connector(connector).ok()??;
        bus.read_gamma(crtc).ok()
    }

    fn write_guarded(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) -> LutRestoreOutcome {
        match self.restore_lut(handle, original, last_value, last_tint) {
            Ok(RestoreOutcome::Restored) => LutRestoreOutcome::Restored,
            Ok(RestoreOutcome::ForeignLutPreserved) => LutRestoreOutcome::ForeignLutPreserved,
            Ok(RestoreOutcome::NothingToRestore) | Err(_) => LutRestoreOutcome::Unavailable,
        }
    }

    fn adopt_baseline(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) {
        let mut session = self.session();
        let entry = session.entry(handle.id().to_string()).or_default();
        if entry.original.is_none() {
            entry.original = Some(original.clone());
            entry.written_value = Some(last_value);
            entry.tint = last_tint;
            entry.written_checksum = Some(original.dimmed(last_value).tinted(last_tint).checksum());
        }
    }
}

fn peak_percent(current: &GammaTable, original: &GammaTable) -> u8 {
    let original_peak = u32::from(original.peak());
    if original_peak == 0 {
        return 0;
    }
    (u32::from(current.peak()) * 100 / original_peak).min(100) as u8
}

pub fn connector_output_suffix(connector: &str) -> Option<&str> {
    let (head, name) = connector.split_once('-')?;
    if head.starts_with("card") && head[4..].parse::<u32>().is_ok() {
        Some(name)
    } else {
        Some(connector)
    }
}

#[cfg(target_os = "linux")]
pub use x11::X11GammaTransport;

#[cfg(target_os = "linux")]
mod x11 {
    use std::fmt;

    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::protocol::randr;

    use super::{connector_output_suffix, GammaBus, GammaError, GammaTable, GammaTransport};
    use x11rb::rust_connection::RustConnection;

    pub struct X11GammaTransport;

    pub struct X11GammaBus {
        conn: RustConnection,
        screen: usize,
    }

    impl GammaTransport for X11GammaTransport {
        type Bus = X11GammaBus;

        fn open(&self) -> Result<Self::Bus, GammaError> {
            let (conn, screen) = x11rb::connect(None).map_err(|error| GammaError::Unsupported {
                detail: format!("cannot connect to the X11 server: {error}"),
            })?;
            Ok(X11GammaBus { conn, screen })
        }
    }

    impl GammaBus for X11GammaBus {
        fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError> {
            self.conn
                .extension_information(randr::X11_EXTENSION_NAME)
                .map_err(x11_error)?
                .ok_or_else(|| GammaError::Unsupported {
                    detail: "the RandR extension is not available on this X11 server".into(),
                })?;
            let Some(root) = self
                .conn
                .setup()
                .roots
                .get(self.screen)
                .map(|screen| screen.root)
            else {
                return Err(GammaError::Unsupported {
                    detail: "the X11 setup carries no screen".into(),
                });
            };
            let resources = randr::get_screen_resources_current(&self.conn, root)
                .map_err(x11_error)?
                .reply()
                .map_err(x11_error)?;
            let Some(suffix) = connector_output_suffix(connector) else {
                return Ok(None);
            };
            for output in resources.outputs {
                let output_info =
                    randr::get_output_info(&self.conn, output, resources.config_timestamp)
                        .map_err(x11_error)?
                        .reply()
                        .map_err(x11_error)?;
                if output_info.connection != randr::Connection::CONNECTED || output_info.crtc == 0 {
                    continue;
                }
                if String::from_utf8_lossy(&output_info.name) == suffix {
                    return Ok(Some(output_info.crtc));
                }
            }
            Ok(None)
        }

        fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError> {
            let reply = randr::get_crtc_gamma(&self.conn, crtc)
                .map_err(x11_error)?
                .reply()
                .map_err(x11_error)?;
            Ok(GammaTable {
                red: reply.red,
                green: reply.green,
                blue: reply.blue,
            })
        }

        fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError> {
            randr::set_crtc_gamma(&self.conn, crtc, &table.red, &table.green, &table.blue)
                .map_err(x11_error)?
                .check()
                .map_err(x11_error)
        }

        fn hdr_active(&mut self, _crtc: u32) -> Result<bool, GammaError> {
            Err(GammaError::Unsupported {
                detail: "HDR state is not readable through RandR on X11".into(),
            })
        }
    }

    fn x11_error(error: impl fmt::Display) -> GammaError {
        GammaError::Unsupported {
            detail: format!("X11 RandR request failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, SessionStore};
    use std::sync::Arc;

    struct FakeGammaBus {
        crtcs: HashMap<String, u32>,
        tables: HashMap<u32, GammaTable>,
        hdr: Result<bool, GammaError>,
        co_owner: Option<GammaTable>,
    }

    impl GammaBus for FakeGammaBus {
        fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError> {
            Ok(self.crtcs.get(connector).copied())
        }

        fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError> {
            self.tables
                .get(&crtc)
                .cloned()
                .ok_or_else(|| GammaError::Unsupported {
                    detail: format!("no gamma table for crtc {crtc}"),
                })
        }

        fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError> {
            self.tables.insert(crtc, table.clone());
            if let Some(co_owner) = &self.co_owner {
                self.tables.insert(crtc, co_owner.clone());
            }
            Ok(())
        }

        fn hdr_active(&mut self, _crtc: u32) -> Result<bool, GammaError> {
            self.hdr.clone()
        }
    }

    struct FakeTransport {
        bus: Arc<Mutex<FakeGammaBus>>,
    }

    impl GammaTransport for FakeTransport {
        type Bus = Arc<Mutex<FakeGammaBus>>;

        fn open(&self) -> Result<Self::Bus, GammaError> {
            Ok(Arc::clone(&self.bus))
        }
    }

    impl GammaBus for Arc<Mutex<FakeGammaBus>> {
        fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError> {
            self.lock().unwrap().crtc_for_connector(connector)
        }

        fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError> {
            self.lock().unwrap().read_gamma(crtc)
        }

        fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError> {
            self.lock().unwrap().write_gamma(crtc, table)
        }

        fn hdr_active(&mut self, crtc: u32) -> Result<bool, GammaError> {
            self.lock().unwrap().hdr_active(crtc)
        }
    }

    fn identity(size: usize, base: u16) -> GammaTable {
        GammaTable {
            red: (0..size).map(|i| base + i as u16).collect(),
            green: (0..size).map(|i| base + 2 * i as u16).collect(),
            blue: (0..size).map(|i| base + 3 * i as u16).collect(),
        }
    }

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".into(), "card0-DP-1".into(), None, false)
    }

    fn backend(bus: FakeGammaBus) -> GammaBackend<FakeTransport> {
        GammaBackend::new(FakeTransport {
            bus: Arc::new(Mutex::new(bus)),
        })
    }

    fn bus(original: &GammaTable, crtc: u32) -> FakeGammaBus {
        FakeGammaBus {
            crtcs: HashMap::from([("card0-DP-1".to_string(), crtc)]),
            tables: HashMap::from([(crtc, original.clone())]),
            hdr: Ok(false),
            co_owner: None,
        }
    }

    #[test]
    fn dimmed_scales_every_channel_and_clamps_at_full() {
        let table = identity(4, 100);
        let half = table.dimmed(50);
        let full = table.dimmed(100);
        let zero = table.dimmed(0);
        assert_eq!(
            half,
            GammaTable {
                red: vec![50, 50, 51, 51],
                green: vec![50, 51, 52, 53],
                blue: vec![50, 51, 53, 54],
            }
        );
        assert_eq!(full, table);
        assert_eq!(
            zero,
            GammaTable {
                red: vec![0; 4],
                green: vec![0; 4],
                blue: vec![0; 4],
            }
        );
    }

    #[test]
    fn dimmed_is_dim_only() {
        let table = identity(4, 100);
        assert_eq!(table.dimmed(101), table.dimmed(100));
    }

    #[test]
    fn neutral_tint_is_identity_and_channels_scale_independently() {
        let table = identity(4, 100);
        assert_eq!(table.tinted(Tint::NEUTRAL), table);
        assert_eq!(
            table.tinted(Tint {
                red: 1000,
                green: 500,
                blue: 250,
            }),
            GammaTable {
                red: vec![100, 101, 102, 103],
                green: vec![50, 51, 52, 53],
                blue: vec![25, 25, 26, 27],
            }
        );
    }

    #[test]
    fn tint_and_brightness_compose_against_the_original_ramp() {
        let original = identity(4, 100);
        let tint = Tint {
            red: 1000,
            green: 700,
            blue: 400,
        };
        let backend = backend(bus(&original, 1));
        backend.set_tint(&handle(), tint).unwrap();
        backend.set_brightness(&handle(), 60).unwrap();
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], original.dimmed(60).tinted(tint));
    }

    #[test]
    fn restore_after_tint_writes_the_original_ramp() {
        let original = identity(4, 100);
        let backend = backend(bus(&original, 1));
        backend
            .set_tint(&handle(), Tint::from_kelvin(3500))
            .unwrap();
        assert_eq!(
            backend.restore(&handle()).unwrap(),
            RestoreOutcome::Restored
        );
        assert_eq!(backend.transport.bus.lock().unwrap().tables[&1], original);
    }

    #[test]
    fn checksum_detects_any_table_change() {
        let table = identity(4, 100);
        let mut changed = table.clone();
        changed.red[1] += 1;
        assert_eq!(table.checksum(), table.checksum());
        assert_ne!(table.checksum(), changed.checksum());
    }

    #[test]
    fn set_captures_the_original_and_writes_a_verified_dim() {
        let backend = backend(bus(&identity(4, 100), 1));
        backend.set_brightness(&handle(), 50).unwrap();
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], identity(4, 100).dimmed(50));
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("id-1").unwrap();
        assert_eq!(entry.original.as_ref().unwrap(), &identity(4, 100));
        assert_eq!(
            entry.written_checksum,
            Some(identity(4, 100).dimmed(50).checksum())
        );
        assert_eq!(entry.mismatches, 0);
    }

    #[test]
    fn set_verifies_by_read_back_and_retries_once() {
        let original = identity(4, 100);
        let mut co_owner = original.clone();
        co_owner.red[0] += 1;
        let backend = backend(bus(&original, 1));
        backend.set_brightness(&handle(), 50).unwrap();
        let mut counts = backend.transport.bus.lock().unwrap();
        counts.co_owner = Some(co_owner);
        drop(counts);
        backend.set_brightness(&handle(), 40).unwrap();
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("id-1").unwrap();
        assert_eq!(
            entry.mismatches, 1,
            "one failed verification round after retry"
        );
        assert_eq!(
            entry.written_checksum,
            Some(original.dimmed(50).checksum()),
            "an unverified write must not become the restore guard"
        );
    }

    #[test]
    fn mismatch_counter_warns_at_three() {
        let original = identity(4, 100);
        let mut co_owner = original.clone();
        co_owner.red[0] += 1;
        let backend = backend(bus(&original, 1));
        backend.set_brightness(&handle(), 90).unwrap();
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.co_owner = Some(co_owner.clone());
        }
        for _ in 0..3 {
            backend.set_brightness(&handle(), 90).unwrap();
        }
        assert_eq!(backend.mismatch_count(&handle()), 3);
        assert!(backend.warned(&handle()));
    }

    #[test]
    fn mismatch_counter_below_three_does_not_warn() {
        let original = identity(4, 100);
        let mut co_owner = original.clone();
        co_owner.red[0] += 1;
        let backend = backend(bus(&original, 1));
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.co_owner = Some(co_owner);
        }
        backend.set_brightness(&handle(), 90).unwrap();
        backend.set_brightness(&handle(), 90).unwrap();
        assert_eq!(backend.mismatch_count(&handle()), 2);
        assert!(!backend.warned(&handle()));
    }

    #[test]
    fn get_returns_the_last_verified_value_and_a_peak_estimate_after_interference() {
        let backend = backend(bus(&identity(4, 100), 1));
        backend.set_brightness(&handle(), 40).unwrap();
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 40);
        assert_eq!(state.source, BrightnessSource::Gamma);
        let gamma = backend.get_gamma(&handle()).unwrap();
        assert_eq!(gamma.value, 40);
        let mut foreign = identity(4, 100).dimmed(40);
        foreign.red[0] += 1;
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.tables.insert(1, foreign);
        }
        let state = backend.get_brightness(&handle()).unwrap();
        let expected = peak_percent(
            &backend.transport.bus.lock().unwrap().tables[&1],
            &identity(4, 100),
        );
        assert_eq!(state.value, expected);
    }

    #[test]
    fn get_before_any_write_reports_full_brightness() {
        let backend = backend(bus(&identity(4, 100), 1));
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 100);
    }

    #[test]
    fn hdr_active_refuses_gamma_with_a_typed_error() {
        let mut active = bus(&identity(4, 100), 1);
        active.hdr = Ok(true);
        let backend = backend(active);
        match backend.set_brightness(&handle(), 50).unwrap_err() {
            MonitorError::Refused { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("HDR"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        assert_eq!(backend.mismatch_count(&handle()), 0);
    }

    #[test]
    fn hdr_unknown_does_not_refuse() {
        let mut unknown = bus(&identity(4, 100), 1);
        unknown.hdr = Err(GammaError::Unsupported {
            detail: "HDR state is not readable through RandR on X11".into(),
        });
        let backend = backend(unknown);
        backend.set_brightness(&handle(), 50).unwrap();
    }

    #[test]
    fn connector_without_an_output_is_typed_unsupported() {
        let mut absent = bus(&identity(4, 100), 1);
        absent.crtcs.clear();
        let backend = backend(absent);
        assert!(matches!(
            backend.get_brightness(&handle()),
            Err(MonitorError::Unsupported {
                capability: "brightness",
                ..
            })
        ));
        let caps = backend.probe(&handle()).unwrap();
        assert!(!caps.brightness_gamma);
        assert!(matches!(
            backend.set_tint(&handle(), Tint::from_kelvin(3500)),
            Err(MonitorError::Unsupported {
                capability: "tint",
                ..
            })
        ));
    }

    #[test]
    fn inert_ramp_is_typed_unsupported() {
        let backend = backend(bus(&identity(1, 65535), 1));
        assert!(matches!(
            backend.set_brightness(&handle(), 50),
            Err(MonitorError::Unsupported { .. })
        ));
        let caps = backend.probe(&handle()).unwrap();
        assert!(!caps.brightness_gamma);
    }

    #[test]
    fn probe_reports_gamma_when_a_real_ramp_is_readable() {
        let backend = backend(bus(&identity(4, 100), 1));
        let caps = backend.probe(&handle()).unwrap();
        assert_eq!(
            caps,
            DisplayCapabilities {
                brightness_gamma: true,
                ..DisplayCapabilities::none()
            }
        );
    }

    #[test]
    fn restore_returns_the_original_table_only_while_our_lut_is_in_place() {
        let original = identity(4, 100);
        let backend = backend(bus(&original, 1));
        assert_eq!(
            backend.restore(&handle()).unwrap(),
            RestoreOutcome::NothingToRestore
        );
        backend.set_brightness(&handle(), 30).unwrap();
        let outcome = backend.restore(&handle()).unwrap();
        assert_eq!(outcome, RestoreOutcome::Restored);
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], original);
        drop(bus);
        assert_eq!(
            backend.restore(&handle()).unwrap(),
            RestoreOutcome::NothingToRestore,
            "restore is idempotent"
        );
    }

    #[test]
    fn restore_preserves_a_foreign_lut() {
        let original = identity(4, 100);
        let mut foreign = original.clone();
        foreign.blue[2] += 1;
        let backend = backend(bus(&original, 1));
        backend.set_brightness(&handle(), 50).unwrap();
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.tables.insert(1, foreign.clone());
        }
        assert_eq!(
            backend.restore(&handle()).unwrap(),
            RestoreOutcome::ForeignLutPreserved
        );
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(
            bus.tables[&1], foreign,
            "a foreign LUT is never overwritten"
        );
    }

    #[test]
    fn restore_write_that_fails_verification_keeps_the_record() {
        let original = identity(4, 100);
        let mut co_owner = original.clone();
        co_owner.red[0] += 1;
        let backend = backend(bus(&original, 1));
        backend.set_brightness(&handle(), 50).unwrap();
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.co_owner = Some(co_owner);
        }
        assert!(matches!(
            backend.restore(&handle()),
            Err(MonitorError::Unsupported { .. })
        ));
        assert_eq!(backend.mismatch_count(&handle()), 1);
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("id-1").unwrap();
        assert_eq!(entry.original.as_ref().unwrap(), &original);
        assert_eq!(
            entry.written_checksum,
            Some(original.dimmed(50).checksum()),
            "an unverified restore keeps the guard for a later attempt"
        );
    }

    #[test]
    fn write_guarded_restores_the_original_without_a_live_session() {
        let original = identity(4, 100);
        let backend = backend(bus(&original.dimmed(50), 1));
        let outcome = backend.write_guarded(&handle(), &original, 50, Tint::NEUTRAL);
        assert_eq!(outcome, LutRestoreOutcome::Restored);
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], original);
    }

    #[test]
    fn write_guarded_preserves_a_foreign_lut_without_a_live_session() {
        let original = identity(4, 100);
        let mut foreign = original.dimmed(50);
        foreign.red[0] += 1;
        let backend = backend(bus(&foreign, 1));
        let outcome = backend.write_guarded(&handle(), &original, 50, Tint::NEUTRAL);
        assert_eq!(outcome, LutRestoreOutcome::ForeignLutPreserved);
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], foreign);
    }

    #[test]
    fn write_guarded_verification_failure_counts_a_mismatch() {
        let original = identity(4, 100);
        let mut co_owner = original.clone();
        co_owner.red[0] += 1;
        let backend = backend(bus(&original.dimmed(50), 1));
        {
            let mut counts = backend.transport.bus.lock().unwrap();
            counts.co_owner = Some(co_owner);
        }
        let outcome = backend.write_guarded(&handle(), &original, 50, Tint::NEUTRAL);
        assert_eq!(outcome, LutRestoreOutcome::Unavailable);
        assert_eq!(backend.mismatch_count(&handle()), 1);
    }

    #[test]
    fn connector_output_suffix_strips_the_card_prefix() {
        assert_eq!(connector_output_suffix("card0-DP-1"), Some("DP-1"));
        assert_eq!(connector_output_suffix("card1-DP-1"), Some("DP-1"));
        assert_eq!(connector_output_suffix("card1-HDMI-A-1"), Some("HDMI-A-1"));
        assert_eq!(connector_output_suffix("card0-eDP-1"), Some("eDP-1"));
        assert_eq!(connector_output_suffix("DP-0"), Some("DP-0"));
        assert_eq!(connector_output_suffix("HDMI-0"), Some("HDMI-0"));
        assert_eq!(connector_output_suffix("card0"), None);
    }

    #[test]
    fn adopt_then_set_dims_against_the_adopted_baseline_not_the_live_crtc() {
        let baseline = identity(4, 100);
        let backend = backend(bus(&baseline.dimmed(80), 1));
        backend.adopt_baseline(&handle(), &baseline, 80, Tint::NEUTRAL);
        backend.set_brightness(&handle(), 80).unwrap();
        let bus = backend.transport.bus.lock().unwrap();
        assert_eq!(bus.tables[&1], baseline.dimmed(80));
        assert_ne!(bus.tables[&1], baseline.dimmed(64));
    }

    #[test]
    fn adopt_baseline_leaves_a_captured_original_alone() {
        let backend = backend(bus(&identity(4, 100), 1));
        backend.set_brightness(&handle(), 50).unwrap();
        let persisted = identity(4, 900);
        backend.adopt_baseline(&handle(), &persisted, 70, Tint::NEUTRAL);
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("id-1").unwrap();
        assert_eq!(entry.original.as_ref().unwrap(), &identity(4, 100));
        assert_eq!(entry.written_value, Some(50));
        assert_eq!(
            entry.written_checksum,
            Some(identity(4, 100).dimmed(50).checksum())
        );
    }

    #[test]
    fn get_brightness_after_adopt_reports_the_persisted_value() {
        let baseline = identity(4, 100);
        let backend = backend(bus(&baseline.dimmed(80), 1));
        backend.adopt_baseline(&handle(), &baseline, 80, Tint::NEUTRAL);
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 80);
        assert_eq!(state.source, BrightnessSource::Gamma);
    }

    #[test]
    fn a_restart_against_a_dimmed_crtc_dims_once_not_twice() {
        let baseline = identity(4, 100);
        let shared = Arc::new(Mutex::new(bus(&baseline, 1)));
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("session"));
        let display = handle();

        let backend1 = Arc::new(GammaBackend::new(FakeTransport {
            bus: Arc::clone(&shared),
        }));
        let session1 = Session::new(backend1.clone(), store.clone(), backend1.clone());
        session1.mutate(&display, 80).unwrap();
        assert_eq!(shared.lock().unwrap().tables[&1], baseline.dimmed(80));
        let persisted = store.load_snapshot("id-1").unwrap().unwrap();
        assert_eq!(persisted.lut.as_ref().unwrap(), &baseline);

        drop(session1);
        drop(backend1);

        let backend2 = Arc::new(GammaBackend::new(FakeTransport {
            bus: Arc::clone(&shared),
        }));
        let session2 = Session::new(backend2.clone(), store.clone(), backend2.clone());
        session2.mutate(&display, 80).unwrap();
        let current = shared.lock().unwrap().tables[&1].clone();
        assert_eq!(current, baseline.dimmed(80));
        assert_ne!(current, baseline.dimmed(64));
        let after = store.load_snapshot("id-1").unwrap().unwrap();
        assert_eq!(after.lut.as_ref().unwrap(), &baseline);
        assert_eq!(after.mutations, 2);
    }
}
