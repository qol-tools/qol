use std::sync::Arc;

use qol_windowing::display::{
    DisplayEnumerator, DisplayHandle, DisplayMode, DisplayOps, DisplayPlacement, DisplaySnapshot,
};

use super::MonitorControl;
use crate::monitor::backends::shared_display::SharedDisplay;
use crate::monitor::night::Tint;
use crate::monitor::{
    BrightnessPolicy, BrightnessState, DdcStatus, DisplayCapabilities, DisplayControl, GammaState,
    GammaStateControl, HdrState, MonitorError, PolicyControl, RestoreOutcome,
};
use crate::session::LutProvider;

pub(crate) struct PlatformControl<D, G, P = qol_windowing::Platform> {
    brightness: PolicyControl<D, G>,
    display: SharedDisplay<P>,
}

impl<D, G, P> PlatformControl<D, G, P> {
    pub(crate) fn new(brightness: PolicyControl<D, G>, display: SharedDisplay<P>) -> Self {
        Self {
            brightness,
            display,
        }
    }
}

impl<D, G, P> DisplayControl for PlatformControl<D, G, P>
where
    D: DisplayControl + DdcStatus + Send + Sync,
    G: DisplayControl + Send + Sync,
    P: DisplayEnumerator + DisplayOps + Send + Sync,
{
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        self.brightness.enumerate()
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, MonitorError> {
        self.display.snapshot()
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let mut capabilities = self.brightness.probe(handle)?;
        capabilities.modes = mode_writes_supported();
        Ok(capabilities)
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        self.brightness.get_brightness(handle)
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.brightness.set_brightness(handle, value)
    }

    fn set_brightness_with_tint(
        &self,
        handle: &DisplayHandle,
        value: u8,
        tint: Tint,
    ) -> Result<(), MonitorError> {
        self.brightness
            .set_brightness_with_tint(handle, value, tint)
    }

    fn set_tint(&self, handle: &DisplayHandle, tint: Tint) -> Result<(), MonitorError> {
        self.brightness.set_tint(handle, tint)
    }

    fn set_gamma_adjustment(
        &self,
        handle: &DisplayHandle,
        value: u8,
        tint: Tint,
    ) -> Result<(), MonitorError> {
        self.brightness.set_gamma_adjustment(handle, value, tint)
    }

    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        self.brightness.get_gamma(handle)
    }

    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.brightness.set_gamma(handle, value)
    }

    fn list_modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        self.display.list_modes(handle)
    }

    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), MonitorError> {
        self.display.set_mode(handle, mode)
    }

    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), MonitorError> {
        self.display.set_layout(placements)
    }

    fn get_hdr(&self, handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        self.brightness.get_hdr(handle)
    }

    fn set_hdr(&self, handle: &DisplayHandle, enabled: bool) -> Result<(), MonitorError> {
        self.brightness.set_hdr(handle, enabled)
    }
}

impl<D, G, P> GammaStateControl for PlatformControl<D, G, P>
where
    D: Send + Sync,
    G: GammaStateControl + Send + Sync,
    P: Send + Sync,
{
    fn mismatch_count(&self, handle: &DisplayHandle) -> usize {
        self.brightness.mismatch_count(handle)
    }

    fn warned(&self, handle: &DisplayHandle) -> bool {
        self.brightness.warned(handle)
    }

    fn restore(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        self.brightness.restore(handle)
    }
}

impl<D, G, P> MonitorControl for PlatformControl<D, G, P>
where
    D: DisplayControl + DdcStatus + Send + Sync,
    G: DisplayControl + GammaStateControl + LutProvider + Send + Sync + 'static,
    P: DisplayEnumerator + DisplayOps + Send + Sync,
{
    fn select(&self, display_id: &str, policy: BrightnessPolicy) {
        self.brightness.select(display_id, policy);
    }

    fn selection(&self, display_id: &str) -> BrightnessPolicy {
        self.brightness.selection(display_id)
    }

    fn gamma_backend(&self) -> Arc<dyn LutProvider> {
        self.brightness.gamma_backend()
    }
}

#[cfg(target_os = "linux")]
fn mode_writes_supported() -> bool {
    super::display_server() == super::DisplayServer::X11
}

#[cfg(target_os = "macos")]
fn mode_writes_supported() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::layout::placements_from_snapshots;
    use crate::monitor::BrightnessSource;
    use qol_windowing::display::DisplayError;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct FakeDdc {
        writes: Arc<Mutex<Vec<(String, u8)>>>,
    }

    impl FakeDdc {
        fn writes(&self) -> Vec<(String, u8)> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl DisplayControl for FakeDdc {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(vec![handle()])
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities {
                brightness_ddc: true,
                ..DisplayCapabilities::none()
            })
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: 21,
                source: BrightnessSource::Ddc,
            })
        }

        fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.writes
                .lock()
                .unwrap()
                .push((handle.id().to_string(), value));
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "fake"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "fake"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "fake"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "fake"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake"))
        }
    }

    impl DdcStatus for FakeDdc {
        fn writes_dropped(&self, _connector: &str) -> bool {
            false
        }
    }

    struct FakeGamma;

    impl DisplayControl for FakeGamma {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(Vec::new())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities::none())
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: 99,
                source: BrightnessSource::Gamma,
            })
        }

        fn set_brightness(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Ok(GammaState { value: 100 })
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Ok(())
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "fake"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "fake"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "fake"))
        }
    }

    #[derive(Clone, Default)]
    struct FakePlatform {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakePlatform {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl DisplayEnumerator for FakePlatform {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
            self.record("enumerate".to_string());
            Err(DisplayError::UnsupportedPlatform)
        }

        fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
            self.record("snapshot".to_string());
            Ok(vec![DisplaySnapshot {
                handle: handle(),
                bounds: qol_windowing::MonitorBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 2560.0,
                    height: 1440.0,
                },
                primary: true,
                mode: Some(mode()),
            }])
        }
    }

    impl DisplayOps for FakePlatform {
        fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
            self.record(format!("modes:{}:{}", handle.id(), handle.connector()));
            Ok(vec![mode()])
        }

        fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError> {
            self.record(format!(
                "set_mode:{}:{}:{}x{}@{}:{}",
                handle.id(),
                handle.connector(),
                mode.width,
                mode.height,
                mode.refresh_hz,
                mode.token
            ));
            Ok(())
        }

        fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
            let described = placements
                .iter()
                .map(|placement| {
                    format!(
                        "{}:{},{}:primary={}:{}",
                        placement.handle.connector(),
                        placement.x,
                        placement.y,
                        placement.primary,
                        placement.handle.id()
                    )
                })
                .collect::<Vec<_>>()
                .join(";");
            self.record(format!("set_layout:{described}"));
            Ok(())
        }
    }

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".to_string(), "card0-DP-1".to_string(), None, false)
    }

    fn mode() -> DisplayMode {
        DisplayMode {
            token: 77,
            width: 2560,
            height: 1440,
            refresh_hz: 144,
        }
    }

    fn control(ddc: FakeDdc) -> PlatformControl<FakeDdc, FakeGamma, FakePlatform> {
        PlatformControl::new(
            PolicyControl::new(ddc, FakeGamma),
            SharedDisplay::with_platform(FakePlatform::default()),
        )
    }

    #[test]
    fn platform_control_delegates_brightness_to_the_policy_control() {
        let ddc = FakeDdc::default();
        let control = control(ddc.clone());
        let handle = handle();
        assert_eq!(control.enumerate().unwrap(), vec![handle.clone()]);
        assert_eq!(
            control.get_brightness(&handle).unwrap(),
            BrightnessState {
                value: 21,
                source: BrightnessSource::Ddc,
            }
        );
        control.set_brightness(&handle, 42).unwrap();
        assert_eq!(ddc.writes(), vec![("id-1".to_string(), 42)]);
        let capabilities = control.probe(&handle).unwrap();
        assert!(capabilities.brightness_ddc);
        assert_eq!(capabilities.modes, mode_writes_supported());
    }

    #[test]
    fn platform_control_forwards_exact_display_arguments_to_the_shared_backend() {
        let platform = FakePlatform::default();
        let control = PlatformControl::new(
            PolicyControl::new(FakeDdc::default(), FakeGamma),
            SharedDisplay::with_platform(platform.clone()),
        );
        let handle = handle();
        let snapshots = control.snapshot().unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].handle.id(), "id-1");
        assert_eq!(snapshots[0].bounds.width, 2560.0);
        let modes = control.list_modes(&handle).unwrap();
        assert_eq!(modes, vec![mode()]);
        control.set_mode(&handle, &modes[0]).unwrap();
        control
            .set_layout(&placements_from_snapshots(&snapshots))
            .unwrap();
        assert_eq!(
            platform.calls(),
            vec![
                "snapshot".to_string(),
                "modes:id-1:card0-DP-1".to_string(),
                "set_mode:id-1:card0-DP-1:2560x1440@144:77".to_string(),
                "set_layout:card0-DP-1:0,0:primary=true:id-1".to_string(),
            ]
        );
    }
}
