use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, GammaStateControl, HdrState, MonitorError, RestoreOutcome, HDR_REASON,
    MODES_REASON,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrightnessPolicy {
    #[default]
    Auto,
    Ddc,
    Gamma,
    Off,
}

impl BrightnessPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ddc => "ddc",
            Self::Gamma => "gamma",
            Self::Off => "off",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "auto" => Some(Self::Auto),
            "ddc" => Some(Self::Ddc),
            "gamma" => Some(Self::Gamma),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

pub trait DdcStatus {
    fn writes_dropped(&self, connector: &str) -> bool;
}

pub struct PolicyControl<D, G> {
    ddc: D,
    gamma: Arc<G>,
    selections: Mutex<HashMap<String, BrightnessPolicy>>,
}

impl<D, G> PolicyControl<D, G> {
    pub fn new(ddc: D, gamma: G) -> Self {
        Self {
            ddc,
            gamma: Arc::new(gamma),
            selections: Mutex::new(HashMap::new()),
        }
    }

    pub fn select(&self, display_id: &str, policy: BrightnessPolicy) {
        self.selections
            .lock()
            .unwrap()
            .insert(display_id.to_string(), policy);
    }

    pub fn selection(&self, display_id: &str) -> BrightnessPolicy {
        self.selections
            .lock()
            .unwrap()
            .get(display_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn gamma_backend(&self) -> Arc<G> {
        Arc::clone(&self.gamma)
    }
}

impl<D: DisplayControl + DdcStatus, G: DisplayControl> PolicyControl<D, G> {
    fn get_auto(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        if self.ddc.writes_dropped(handle.connector()) {
            return self.gamma.get_brightness(handle);
        }
        match self.ddc.get_brightness(handle) {
            Ok(state) => Ok(BrightnessState {
                value: state.value,
                source: BrightnessSource::Ddc,
            }),
            Err(_) => self.gamma.get_brightness(handle),
        }
    }

    fn set_auto(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        if self.ddc.writes_dropped(handle.connector()) {
            return self.gamma.set_brightness(handle, value);
        }
        match self.ddc.set_brightness(handle, value) {
            Ok(()) => {
                if self.ddc.writes_dropped(handle.connector()) {
                    self.gamma.set_brightness(handle, value)
                } else {
                    Ok(())
                }
            }
            Err(_) => self.gamma.set_brightness(handle, value),
        }
    }

    fn combined_caps(&self, handle: &DisplayHandle) -> DisplayCapabilities {
        let ddc = self
            .ddc
            .probe(handle)
            .unwrap_or_else(|_| DisplayCapabilities::none());
        let gamma = self
            .gamma
            .probe(handle)
            .unwrap_or_else(|_| DisplayCapabilities::none());
        DisplayCapabilities {
            brightness_ddc: ddc.brightness_ddc,
            brightness_gamma: gamma.brightness_gamma,
            ..DisplayCapabilities::none()
        }
    }
}

impl<D: DisplayControl + DdcStatus, G: DisplayControl> DisplayControl for PolicyControl<D, G> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        self.ddc.enumerate()
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        match self.selection(handle.id()) {
            BrightnessPolicy::Ddc => self.ddc.probe(handle),
            BrightnessPolicy::Gamma => self.gamma.probe(handle),
            BrightnessPolicy::Auto | BrightnessPolicy::Off => Ok(self.combined_caps(handle)),
        }
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        match self.selection(handle.id()) {
            BrightnessPolicy::Off => Err(MonitorError::refused(
                "brightness",
                "control is off for this display",
            )),
            BrightnessPolicy::Gamma => self.gamma.get_brightness(handle),
            BrightnessPolicy::Ddc => self
                .ddc
                .get_brightness(handle)
                .map(|state| BrightnessState {
                    value: state.value,
                    source: BrightnessSource::Ddc,
                }),
            BrightnessPolicy::Auto => self.get_auto(handle),
        }
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        match self.selection(handle.id()) {
            BrightnessPolicy::Off => Err(MonitorError::refused(
                "brightness",
                "control is off for this display",
            )),
            BrightnessPolicy::Gamma => self.gamma.set_brightness(handle, value),
            BrightnessPolicy::Ddc => self.ddc.set_brightness(handle, value),
            BrightnessPolicy::Auto => self.set_auto(handle, value),
        }
    }

    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        self.gamma.get_gamma(handle)
    }

    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.gamma.set_gamma(handle, value)
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

impl<D: Send + Sync, G: GammaStateControl> GammaStateControl for PolicyControl<D, G> {
    fn mismatch_count(&self, handle: &DisplayHandle) -> usize {
        self.gamma.mismatch_count(handle)
    }

    fn warned(&self, handle: &DisplayHandle) -> bool {
        self.gamma.warned(handle)
    }

    fn restore(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        self.gamma.restore(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn handle(id: &str, connector: &str) -> DisplayHandle {
        DisplayHandle::new(id.into(), connector.into(), None, false)
    }

    #[derive(Clone, Copy)]
    enum FakeError {
        Permission,
    }

    impl FakeError {
        fn monitor(&self) -> MonitorError {
            match self {
                Self::Permission => MonitorError::I2c(crate::monitor::I2cError::Permission {
                    node: "/dev/i2c-7".into(),
                }),
            }
        }
    }

    struct FakeDdc {
        sets: Arc<Mutex<usize>>,
        gets: Arc<Mutex<usize>>,
        value: Arc<Mutex<u8>>,
        dropped: Arc<Mutex<bool>>,
        set_error: Option<FakeError>,
        get_error: Option<FakeError>,
        probe_error: Option<FakeError>,
    }

    impl FakeDdc {
        fn healthy(value: u8) -> Self {
            Self {
                sets: Arc::new(Mutex::new(0)),
                gets: Arc::new(Mutex::new(0)),
                value: Arc::new(Mutex::new(value)),
                dropped: Arc::new(Mutex::new(false)),
                set_error: None,
                get_error: None,
                probe_error: None,
            }
        }

        fn set_counts(&self) -> usize {
            *self.sets.lock().unwrap()
        }

        fn value(&self) -> u8 {
            *self.value.lock().unwrap()
        }
    }

    impl DisplayControl for FakeDdc {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(vec![])
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            match &self.probe_error {
                Some(error) => Err(error.monitor()),
                None => Ok(DisplayCapabilities {
                    brightness_ddc: !*self.dropped.lock().unwrap(),
                    ..DisplayCapabilities::none()
                }),
            }
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            *self.gets.lock().unwrap() += 1;
            match &self.get_error {
                Some(error) => Err(error.monitor()),
                None => Ok(BrightnessState {
                    value: *self.value.lock().unwrap(),
                    source: BrightnessSource::Ddc,
                }),
            }
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            *self.sets.lock().unwrap() += 1;
            match &self.set_error {
                Some(error) => Err(error.monitor()),
                None => {
                    *self.value.lock().unwrap() = value;
                    Ok(())
                }
            }
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "fake ddc"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "fake ddc"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "fake ddc"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "fake ddc"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake ddc"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake ddc"))
        }
    }

    impl DdcStatus for FakeDdc {
        fn writes_dropped(&self, _connector: &str) -> bool {
            *self.dropped.lock().unwrap()
        }
    }

    #[derive(Clone)]
    struct FakeGamma {
        sets: Arc<Mutex<usize>>,
        value: Arc<Mutex<u8>>,
        mismatch: Arc<Mutex<usize>>,
        warned: Arc<Mutex<bool>>,
        restore_outcome: Arc<Mutex<RestoreOutcome>>,
        restore_calls: Arc<Mutex<usize>>,
    }

    impl FakeGamma {
        fn new() -> Self {
            Self {
                sets: Arc::new(Mutex::new(0)),
                value: Arc::new(Mutex::new(100)),
                mismatch: Arc::new(Mutex::new(0)),
                warned: Arc::new(Mutex::new(false)),
                restore_outcome: Arc::new(Mutex::new(RestoreOutcome::Restored)),
                restore_calls: Arc::new(Mutex::new(0)),
            }
        }

        fn set_counts(&self) -> usize {
            *self.sets.lock().unwrap()
        }

        fn value(&self) -> u8 {
            *self.value.lock().unwrap()
        }
    }

    impl DisplayControl for FakeGamma {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(vec![])
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities {
                brightness_gamma: true,
                ..DisplayCapabilities::none()
            })
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: *self.value.lock().unwrap(),
                source: BrightnessSource::Gamma,
            })
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            *self.sets.lock().unwrap() += 1;
            *self.value.lock().unwrap() = value;
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Ok(GammaState {
                value: *self.value.lock().unwrap(),
            })
        }

        fn set_gamma(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            *self.sets.lock().unwrap() += 1;
            *self.value.lock().unwrap() = value;
            Ok(())
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "fake gamma"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "fake gamma"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake gamma"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake gamma"))
        }
    }

    impl GammaStateControl for FakeGamma {
        fn mismatch_count(&self, _handle: &DisplayHandle) -> usize {
            *self.mismatch.lock().unwrap()
        }

        fn warned(&self, _handle: &DisplayHandle) -> bool {
            *self.warned.lock().unwrap()
        }

        fn restore(&self, _handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
            *self.restore_calls.lock().unwrap() += 1;
            Ok(*self.restore_outcome.lock().unwrap())
        }
    }

    fn policy(ddc: FakeDdc, gamma: FakeGamma) -> PolicyControl<FakeDdc, FakeGamma> {
        PolicyControl::new(ddc, gamma)
    }

    #[test]
    fn policy_labels_parse_and_round_trip() {
        for (label, policy) in [
            ("auto", BrightnessPolicy::Auto),
            ("ddc", BrightnessPolicy::Ddc),
            ("gamma", BrightnessPolicy::Gamma),
            ("off", BrightnessPolicy::Off),
        ] {
            assert_eq!(BrightnessPolicy::parse(label), Some(policy));
            assert_eq!(policy.label(), label);
        }
        assert_eq!(BrightnessPolicy::parse("night"), None);
        assert_eq!(BrightnessPolicy::default(), BrightnessPolicy::Auto);
    }

    #[test]
    fn auto_prefers_ddc_when_it_works() {
        let display = handle("id-1", "card0-DP-1");
        let ddc = FakeDdc::healthy(42);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.value, 42);
        assert_eq!(state.source, BrightnessSource::Ddc);
        control.set_brightness(&display, 50).unwrap();
        assert_eq!(control.ddc.set_counts(), 1);
        assert_eq!(control.gamma.set_counts(), 0);
        assert_eq!(control.ddc.value(), 50);
        let caps = control.probe(&display).unwrap();
        assert!(caps.brightness_ddc);
        assert!(caps.brightness_gamma);
    }

    #[test]
    fn auto_falls_back_to_gamma_when_ddc_reads_fail() {
        let display = handle("id-1", "card0-DP-1");
        let mut ddc = FakeDdc::healthy(42);
        ddc.get_error = Some(FakeError::Permission);
        ddc.set_error = Some(FakeError::Permission);
        ddc.probe_error = Some(FakeError::Permission);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.value, 100);
        assert_eq!(state.source, BrightnessSource::Gamma);
        control.set_brightness(&display, 30).unwrap();
        assert_eq!(control.gamma.set_counts(), 1);
        assert_eq!(control.gamma.value(), 30);
        let caps = control.probe(&display).unwrap();
        assert!(!caps.brightness_ddc);
        assert!(caps.brightness_gamma);
    }

    #[test]
    fn auto_engages_gamma_when_ddc_writes_are_dropped() {
        let display = handle("id-1", "card0-DP-1");
        let ddc = FakeDdc::healthy(20);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        *control.ddc.dropped.lock().unwrap() = true;
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.source, BrightnessSource::Gamma);
        control.set_brightness(&display, 40).unwrap();
        assert_eq!(control.ddc.set_counts(), 0, "dropped DDC is never written");
        assert_eq!(control.gamma.set_counts(), 1);
    }

    #[test]
    fn auto_switches_to_gamma_on_the_set_that_gets_dropped() {
        let display = handle("id-1", "card0-DP-1");
        let ddc = FakeDdc::healthy(20);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        control.set_brightness(&display, 50).unwrap();
        assert_eq!(control.ddc.set_counts(), 1);
        assert_eq!(control.gamma.set_counts(), 0);
        *control.ddc.dropped.lock().unwrap() = true;
        control.set_brightness(&display, 60).unwrap();
        assert_eq!(
            control.ddc.set_counts(),
            1,
            "the drop is detected after the set"
        );
        assert_eq!(
            control.gamma.set_counts(),
            1,
            "the dropped set engages gamma"
        );
        assert_eq!(control.gamma.value(), 60);
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.value, 60);
        assert_eq!(
            state.source,
            BrightnessSource::Gamma,
            "the source label flips"
        );
        control.set_brightness(&display, 70).unwrap();
        assert_eq!(
            control.gamma.set_counts(),
            2,
            "gamma owns the display from then on"
        );
    }

    #[test]
    fn ddc_policy_surfaces_errors_never_falls_back() {
        let display = handle("id-1", "card0-DP-1");
        let mut ddc = FakeDdc::healthy(42);
        ddc.set_error = Some(FakeError::Permission);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        control.select("id-1", BrightnessPolicy::Ddc);
        assert!(control.set_brightness(&display, 50).is_err());
        assert_eq!(
            control.gamma.set_counts(),
            0,
            "no silent fallback under ddc"
        );
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.source, BrightnessSource::Ddc);
    }

    #[test]
    fn ddc_policy_probe_failures_surface() {
        let display = handle("id-1", "card0-DP-1");
        let mut ddc = FakeDdc::healthy(42);
        ddc.probe_error = Some(FakeError::Permission);
        let control = policy(ddc, FakeGamma::new());
        control.select("id-1", BrightnessPolicy::Ddc);
        assert!(matches!(
            control.probe(&display),
            Err(MonitorError::I2c(
                crate::monitor::I2cError::Permission { .. }
            ))
        ));
    }

    #[test]
    fn gamma_policy_routes_only_gamma() {
        let display = handle("id-1", "card0-DP-1");
        let ddc = FakeDdc::healthy(42);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        control.select("id-1", BrightnessPolicy::Gamma);
        control.set_brightness(&display, 25).unwrap();
        assert_eq!(control.ddc.set_counts(), 0);
        assert_eq!(control.gamma.set_counts(), 1);
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.source, BrightnessSource::Gamma);
        assert_eq!(state.value, 25);
        let caps = control.probe(&display).unwrap();
        assert!(!caps.brightness_ddc);
        assert!(caps.brightness_gamma);
    }

    #[test]
    fn off_policy_refuses_get_and_set() {
        let display = handle("id-1", "card0-DP-1");
        let control = policy(FakeDdc::healthy(42), FakeGamma::new());
        control.select("id-1", BrightnessPolicy::Off);
        match control.set_brightness(&display, 50).unwrap_err() {
            MonitorError::Refused { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("off"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        assert!(matches!(
            control.get_brightness(&display),
            Err(MonitorError::Refused { .. })
        ));
        let caps = control.probe(&display).unwrap();
        assert!(
            caps.brightness_ddc,
            "off is a control choice, not a capability"
        );
        assert!(caps.brightness_gamma);
    }

    #[test]
    fn off_is_a_per_display_opt_out() {
        let display = handle("id-1", "card0-DP-1");
        let other = handle("id-2", "card0-DP-2");
        let control = policy(FakeDdc::healthy(42), FakeGamma::new());
        control.select("id-1", BrightnessPolicy::Off);
        control.set_brightness(&other, 50).unwrap();
        assert_eq!(control.ddc.set_counts(), 1);
        control.set_brightness(&display, 50).unwrap_err();
    }

    #[test]
    fn selection_defaults_to_auto_and_is_per_display() {
        let control = policy(FakeDdc::healthy(42), FakeGamma::new());
        assert_eq!(control.selection("id-1"), BrightnessPolicy::Auto);
        control.select("id-1", BrightnessPolicy::Gamma);
        assert_eq!(control.selection("id-1"), BrightnessPolicy::Gamma);
        assert_eq!(control.selection("id-2"), BrightnessPolicy::Auto);
    }

    #[test]
    fn gamma_state_and_restore_delegate_through_the_policy() {
        let display = handle("id-1", "card0-DP-1");
        let ddc = FakeDdc::healthy(42);
        let gamma = FakeGamma::new();
        let control = policy(ddc, gamma);
        *control.gamma.mismatch.lock().unwrap() = 3;
        *control.gamma.warned.lock().unwrap() = true;
        *control.gamma.restore_outcome.lock().unwrap() = RestoreOutcome::ForeignLutPreserved;
        assert_eq!(control.mismatch_count(&display), 3);
        assert!(control.warned(&display));
        assert_eq!(
            control.restore(&display).unwrap(),
            RestoreOutcome::ForeignLutPreserved
        );
        assert_eq!(*control.gamma.restore_calls.lock().unwrap(), 1);
    }

    #[test]
    fn explicit_gamma_calls_route_to_the_gamma_backend() {
        let display = handle("id-1", "card0-DP-1");
        let gamma = FakeGamma::new();
        let control = policy(FakeDdc::healthy(42), gamma);
        control.set_gamma(&display, 33).unwrap();
        assert_eq!(control.gamma.set_counts(), 1);
        assert_eq!(control.gamma.value(), 33);
        assert_eq!(control.get_gamma(&display).unwrap().value, 33);
    }

    struct FakeI2cMonitor {
        current: u16,
        max: u16,
        drop_writes: bool,
    }

    struct FakeI2cBus {
        monitor: Arc<Mutex<FakeI2cMonitor>>,
        pending: Option<Vec<u8>>,
    }

    impl crate::monitor::backends::i2c_ddc::I2cBus for FakeI2cBus {
        fn write(&mut self, frame: &[u8]) -> Result<(), crate::monitor::I2cError> {
            let mut monitor = self.monitor.lock().unwrap();
            let length = frame[1];
            if length != 0x82 && length != 0x84 {
                return Err(crate::monitor::I2cError::Protocol {
                    detail: "request carries an unknown DDC/CI length byte".into(),
                });
            }
            let request_checksum = frame[..frame.len() - 1]
                .iter()
                .fold(0x6e, |acc, byte| acc ^ byte);
            if request_checksum != frame[frame.len() - 1] {
                return Err(crate::monitor::I2cError::Protocol {
                    detail: "request checksum mismatch".into(),
                });
            }
            match frame[2] {
                0x03 => {
                    let value = u16::from_be_bytes([frame[4], frame[5]]);
                    if !monitor.drop_writes {
                        monitor.current = value;
                    }
                }
                0x01 => {
                    let mut reply = [0u8; 11];
                    reply[0] = 0x6e;
                    reply[1] = 0x88;
                    reply[2] = 0x02;
                    reply[4] = frame[3];
                    reply[6] = (monitor.max >> 8) as u8;
                    reply[7] = monitor.max as u8;
                    reply[8] = (monitor.current >> 8) as u8;
                    reply[9] = monitor.current as u8;
                    reply[10] = reply[1..10].iter().fold(0x50, |acc, byte| acc ^ byte);
                    self.pending = Some(reply.to_vec());
                }
                _ => {}
            }
            Ok(())
        }

        fn read(&mut self, buffer: &mut [u8]) -> Result<usize, crate::monitor::I2cError> {
            let Some(pending) = self.pending.take() else {
                return Ok(0);
            };
            let count = pending.len().min(buffer.len());
            buffer[..count].copy_from_slice(&pending[..count]);
            Ok(count)
        }
    }

    struct FakeI2cTransport {
        monitor: Arc<Mutex<FakeI2cMonitor>>,
    }

    impl crate::monitor::backends::i2c_ddc::I2cTransport for FakeI2cTransport {
        type Bus = FakeI2cBus;

        fn open(&self, _dev: &std::path::Path) -> Result<Self::Bus, crate::monitor::I2cError> {
            Ok(FakeI2cBus {
                monitor: Arc::clone(&self.monitor),
                pending: None,
            })
        }
    }

    fn sysfs_with_links(links: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        std::fs::create_dir_all(&connector_dir).unwrap();
        let adapters = dir.path().join("adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        for (link, name) in links {
            let adapter = adapters.join(link);
            std::fs::create_dir_all(&adapter).unwrap();
            std::fs::write(adapter.join("name"), name).unwrap();
            std::os::unix::fs::symlink(format!("../adapters/{link}"), connector_dir.join(link))
                .unwrap();
        }
        dir
    }

    #[test]
    fn a2a_dropped_writes_engage_the_gamma_backend_end_to_end() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let monitor = Arc::new(Mutex::new(FakeI2cMonitor {
            current: 200,
            max: 1000,
            drop_writes: true,
        }));
        let ddc = crate::monitor::backends::i2c_ddc::I2cDdcBackend::with_timing(
            FakeI2cTransport { monitor },
            sysfs.path().to_path_buf(),
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        );
        let gamma = FakeGamma::new();
        let control = PolicyControl::new(ddc, gamma);
        let display = handle("id-1", "card0-DP-1");
        control.set_brightness(&display, 50).unwrap();
        assert_eq!(
            control.gamma.set_counts(),
            1,
            "the dropped DDC set engages the gamma backend in the same call"
        );
        assert_eq!(control.gamma.value(), 50);
        let state = control.get_brightness(&display).unwrap();
        assert_eq!(state.source, BrightnessSource::Gamma);
        assert_eq!(state.value, 50);
        control.set_brightness(&display, 70).unwrap();
        assert_eq!(
            control.gamma.set_counts(),
            2,
            "gamma owns the display from then on"
        );
        let caps = control.probe(&display).unwrap();
        assert!(!caps.brightness_ddc);
        assert!(caps.brightness_gamma);
    }
}
