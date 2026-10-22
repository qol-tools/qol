use std::sync::Arc;

use crate::monitor::policy::DdcStatus;
use crate::monitor::{
    BrightnessPolicy, DisplayControl, GammaStateControl, PolicyControl, StubControl,
};
use crate::session::{LutProvider, NoLutProvider};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod platform_control;
mod support;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_signals;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{
    brightness_floor, control, current_support, fit_tint, install_signal_handlers, native_desktop,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    brightness_floor, control, current_support, fit_tint, install_signal_handlers, native_desktop,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    brightness_floor, control, current_support, fit_tint, install_signal_handlers, native_desktop,
};
pub(crate) use support::PlatformSupport;
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    brightness_floor, control, current_support, fit_tint, install_signal_handlers, native_desktop,
};

pub(crate) trait MonitorControl: DisplayControl + GammaStateControl {
    fn select(&self, display_id: &str, policy: BrightnessPolicy);
    fn selection(&self, display_id: &str) -> BrightnessPolicy;
    fn gamma_backend(&self) -> Arc<dyn LutProvider>;
}

pub(crate) type Control = Arc<dyn MonitorControl>;

impl MonitorControl for StubControl {
    fn select(&self, _display_id: &str, _policy: BrightnessPolicy) {}

    fn selection(&self, _display_id: &str) -> BrightnessPolicy {
        BrightnessPolicy::Auto
    }

    fn gamma_backend(&self) -> Arc<dyn LutProvider> {
        Arc::new(NoLutProvider)
    }
}

impl<D, G> MonitorControl for PolicyControl<D, G>
where
    D: DisplayControl + DdcStatus + Send + Sync,
    G: DisplayControl + GammaStateControl + LutProvider + Send + Sync + 'static,
{
    fn select(&self, display_id: &str, policy: BrightnessPolicy) {
        PolicyControl::select(self, display_id, policy);
    }

    fn selection(&self, display_id: &str) -> BrightnessPolicy {
        PolicyControl::selection(self, display_id)
    }

    fn gamma_backend(&self) -> Arc<dyn LutProvider> {
        self.gamma_backend()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayServer {
    X11,
    Wayland,
    Desktop,
    None,
}

impl DisplayServer {
    fn from_env() -> Self {
        match qol_platform::linux_display_backend() {
            qol_platform::LinuxDisplayBackend::Wayland => Self::Wayland,
            qol_platform::LinuxDisplayBackend::X11 => Self::X11,
            qol_platform::LinuxDisplayBackend::Unknown => Self::None,
        }
    }
}

pub(crate) fn display_server() -> DisplayServer {
    if native_desktop() {
        DisplayServer::Desktop
    } else {
        DisplayServer::from_env()
    }
}

pub(crate) fn apply_configured_policies(control: &Control, device: &crate::config::DeviceConfig) {
    let stable_ids: std::collections::HashSet<String> = control
        .enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|handle| !handle.identity_unstable())
        .map(|handle| handle.id().to_string())
        .collect();
    for display_id in stable_ids {
        control.select(&display_id, device.policy_for(&display_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_facade_selects_nothing_and_offers_no_lut() {
        let control: Control = Arc::new(StubControl);
        control.select("id-1", BrightnessPolicy::Gamma);
        assert!(control.gamma_backend().capture("card0-DP-1").is_none());
    }
}
