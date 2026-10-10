use std::collections::HashSet;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use qol_windowing::DisplayEnumerator;

use crate::monitor::backends::i2c_ddc::I2cError;
use crate::monitor::policy::DdcStatus;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, HdrState, MonitorError, GAMMA_REASON, HDR_REASON, MODES_REASON,
};

pub const FEATURE_BRIGHTNESS: u8 = 0x10;
const WRITE_ATTEMPTS: usize = 2;
const SETTLE_DELAY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpReading {
    pub current: u32,
    pub max: u32,
}

pub trait VcpTransport: Send + Sync {
    fn read(&self, connector: &str, code: u8) -> Result<VcpReading, I2cError>;
    fn write(&self, connector: &str, code: u8, value: u32) -> Result<(), I2cError>;
}

pub struct VcpDdcBackend<T: VcpTransport, E: DisplayEnumerator> {
    transport: T,
    displays: E,
    settle: Duration,
    dropped_writes: Mutex<HashSet<String>>,
}

impl<T: VcpTransport, E: DisplayEnumerator> VcpDdcBackend<T, E> {
    pub fn new(transport: T, displays: E) -> Self {
        Self::with_settle(transport, displays, SETTLE_DELAY)
    }

    pub fn with_settle(transport: T, displays: E, settle: Duration) -> Self {
        Self {
            transport,
            displays,
            settle,
            dropped_writes: Mutex::new(HashSet::new()),
        }
    }

    fn writes_dropped(&self, connector: &str) -> bool {
        self.dropped_writes.lock().unwrap().contains(connector)
    }

    fn get_brightness_inner(&self, handle: &DisplayHandle) -> Result<u8, I2cError> {
        percent_from_reading(
            self.transport
                .read(handle.connector(), FEATURE_BRIGHTNESS)?,
        )
    }

    fn set_brightness_inner(&self, handle: &DisplayHandle, value: u8) -> Result<(), I2cError> {
        let connector = handle.connector();
        if self.writes_dropped(connector) {
            return Err(I2cError::UnsupportedTransport {
                detail: format!(
                    "DDC/CI writes were dropped on {connector}; set the display policy to gamma \
                     to keep brightness control"
                ),
            });
        }
        let reading = self.transport.read(connector, FEATURE_BRIGHTNESS)?;
        if reading.max == 0 {
            return Err(I2cError::Protocol {
                detail: format!("{connector} reports a maximum brightness of 0"),
            });
        }
        let target = raw_from_percent(value, reading.max);
        let mut verified = false;
        for _ in 0..WRITE_ATTEMPTS {
            self.transport
                .write(connector, FEATURE_BRIGHTNESS, target)?;
            thread::sleep(self.settle);
            if self.transport.read(connector, FEATURE_BRIGHTNESS)?.current == target {
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

pub fn percent_from_reading(reading: VcpReading) -> Result<u8, I2cError> {
    if reading.max == 0 {
        return Err(I2cError::Protocol {
            detail: "the display reports a maximum brightness of 0".into(),
        });
    }
    let percent = u64::from(reading.current) * 100 / u64::from(reading.max);
    Ok(percent.min(100) as u8)
}

pub fn raw_from_percent(value: u8, max: u32) -> u32 {
    (u64::from(value.min(100)) * u64::from(max) / 100) as u32
}

impl<T: VcpTransport, E: DisplayEnumerator + Send + Sync> DisplayControl for VcpDdcBackend<T, E> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(self.displays.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        if self.writes_dropped(handle.connector()) {
            return Ok(DisplayCapabilities::none());
        }
        Ok(DisplayCapabilities {
            brightness_ddc: self.get_brightness_inner(handle).is_ok(),
            ..DisplayCapabilities::none()
        })
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

impl<T: VcpTransport, E: DisplayEnumerator> DdcStatus for VcpDdcBackend<T, E> {
    fn writes_dropped(&self, connector: &str) -> bool {
        self.writes_dropped(connector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use qol_windowing::display::{DisplayError, DisplaySnapshot};

    #[derive(Default)]
    struct FakeMonitor {
        current: u32,
        max: u32,
        drop_writes: bool,
        unreachable: bool,
        writes: Vec<(String, u8, u32)>,
    }

    #[derive(Clone)]
    struct FakeTransport(Arc<Mutex<FakeMonitor>>);

    impl VcpTransport for FakeTransport {
        fn read(&self, connector: &str, code: u8) -> Result<VcpReading, I2cError> {
            let monitor = self.0.lock().unwrap();
            if monitor.unreachable {
                return Err(I2cError::UnsupportedTransport {
                    detail: format!("{connector} answers no VCP {code:#04x} request"),
                });
            }
            Ok(VcpReading {
                current: monitor.current,
                max: monitor.max,
            })
        }

        fn write(&self, connector: &str, code: u8, value: u32) -> Result<(), I2cError> {
            let mut monitor = self.0.lock().unwrap();
            monitor.writes.push((connector.to_string(), code, value));
            if !monitor.drop_writes {
                monitor.current = value;
            }
            Ok(())
        }
    }

    struct FakeDisplays;

    impl DisplayEnumerator for FakeDisplays {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
            Ok(vec![handle()])
        }

        fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
            Ok(Vec::new())
        }
    }

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".into(), "DISPLAY1".into(), None, false)
    }

    fn backend(
        monitor: FakeMonitor,
    ) -> (VcpDdcBackend<FakeTransport, FakeDisplays>, FakeTransport) {
        let transport = FakeTransport(Arc::new(Mutex::new(monitor)));
        let backend = VcpDdcBackend::with_settle(transport.clone(), FakeDisplays, Duration::ZERO);
        (backend, transport)
    }

    #[test]
    fn percent_from_reading_scales_and_clamps() {
        let cases = [
            (0, 100, Some(0)),
            (50, 100, Some(50)),
            (100, 100, Some(100)),
            (127, 255, Some(49)),
            (255, 255, Some(100)),
            (500, 1000, Some(50)),
            (120, 100, Some(100)),
            (10, 0, None),
        ];
        for (current, max, expected) in cases {
            let actual = percent_from_reading(VcpReading { current, max }).ok();
            assert_eq!(actual, expected, "current {current} max {max}");
        }
    }

    #[test]
    fn raw_from_percent_scales_into_the_monitor_range() {
        let cases = [
            (0, 100, 0),
            (50, 100, 50),
            (100, 100, 100),
            (50, 255, 127),
            (100, 255, 255),
            (35, 1000, 350),
            (150, 100, 100),
            (70, u32::MAX, 3_006_477_106),
        ];
        for (value, max, expected) in cases {
            assert_eq!(
                raw_from_percent(value, max),
                expected,
                "value {value} max {max}"
            );
        }
    }

    #[test]
    fn set_brightness_writes_the_raw_value_and_tracks_dropped_writes() {
        struct Case {
            max: u32,
            drop_writes: bool,
            value: u8,
            writes: Vec<u32>,
            dropped: bool,
        }
        let cases = [
            Case {
                max: 100,
                drop_writes: false,
                value: 40,
                writes: vec![40],
                dropped: false,
            },
            Case {
                max: 255,
                drop_writes: false,
                value: 50,
                writes: vec![127],
                dropped: false,
            },
            Case {
                max: 100,
                drop_writes: true,
                value: 40,
                writes: vec![40, 40],
                dropped: true,
            },
        ];
        for case in cases {
            let (backend, transport) = backend(FakeMonitor {
                current: 80,
                max: case.max,
                drop_writes: case.drop_writes,
                ..FakeMonitor::default()
            });
            backend.set_brightness(&handle(), case.value).unwrap();
            let writes = transport
                .0
                .lock()
                .unwrap()
                .writes
                .iter()
                .map(|(connector, code, value)| {
                    assert_eq!(connector, "DISPLAY1");
                    assert_eq!(*code, FEATURE_BRIGHTNESS);
                    *value
                })
                .collect::<Vec<_>>();
            assert_eq!(writes, case.writes, "max {} value {}", case.max, case.value);
            assert_eq!(
                DdcStatus::writes_dropped(&backend, "DISPLAY1"),
                case.dropped
            );
            let source = backend.get_brightness(&handle()).unwrap().source;
            let expected = if case.dropped {
                BrightnessSource::Gamma
            } else {
                BrightnessSource::Ddc
            };
            assert_eq!(source, expected);
        }
    }

    #[test]
    fn a_dropped_display_refuses_further_writes_and_loses_the_capability() {
        let (backend, transport) = backend(FakeMonitor {
            current: 80,
            max: 100,
            drop_writes: true,
            ..FakeMonitor::default()
        });
        backend.set_brightness(&handle(), 40).unwrap();
        let writes_before = transport.0.lock().unwrap().writes.len();
        let error = backend.set_brightness(&handle(), 60).unwrap_err();
        assert!(matches!(error, MonitorError::Unsupported { .. }), "{error}");
        assert_eq!(transport.0.lock().unwrap().writes.len(), writes_before);
        assert!(!backend.probe(&handle()).unwrap().brightness_ddc);
    }

    #[test]
    fn probe_and_reads_map_monitor_answers() {
        let cases = [
            (false, 100, true, Some(30)),
            (true, 100, false, None),
            (false, 0, false, None),
        ];
        for (unreachable, max, capable, value) in cases {
            let (backend, _) = backend(FakeMonitor {
                current: 30,
                max,
                unreachable,
                ..FakeMonitor::default()
            });
            assert_eq!(
                backend.probe(&handle()).unwrap().brightness_ddc,
                capable,
                "unreachable {unreachable} max {max}"
            );
            assert_eq!(
                backend
                    .get_brightness(&handle())
                    .ok()
                    .map(|state| state.value),
                value
            );
        }
    }

    #[test]
    fn a_zero_maximum_refuses_the_write() {
        let (backend, transport) = backend(FakeMonitor {
            current: 0,
            max: 0,
            ..FakeMonitor::default()
        });
        let error = backend.set_brightness(&handle(), 50).unwrap_err();
        assert!(matches!(
            error,
            MonitorError::I2c(I2cError::Protocol { .. })
        ));
        assert!(transport.0.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn enumerate_comes_from_the_display_enumerator() {
        let (backend, _) = backend(FakeMonitor::default());
        assert_eq!(backend.enumerate().unwrap(), vec![handle()]);
    }
}
