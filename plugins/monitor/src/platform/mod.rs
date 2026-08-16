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
mod support;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{control, current_support};
#[cfg(target_os = "linux")]
pub(crate) use linux::{control, current_support};
#[cfg(target_os = "macos")]
pub(crate) use macos::{control, current_support};
pub(crate) use support::PlatformSupport;
#[cfg(target_os = "windows")]
pub(crate) use windows::{control, current_support};

pub(crate) trait MonitorControl: DisplayControl + GammaStateControl {
    fn select(&self, display_id: &str, policy: BrightnessPolicy);
    fn gamma_backend(&self) -> Arc<dyn LutProvider>;
}

pub(crate) type Control = Arc<dyn MonitorControl>;

impl MonitorControl for StubControl {
    fn select(&self, _display_id: &str, _policy: BrightnessPolicy) {}

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

    fn gamma_backend(&self) -> Arc<dyn LutProvider> {
        self.gamma_backend()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayServer {
    X11,
    Wayland,
    None,
}

impl DisplayServer {
    pub(crate) fn detect(
        wayland_display: Option<&str>,
        session_type: Option<&str>,
        display: Option<&str>,
    ) -> Self {
        if wayland_display.is_some() || session_type == Some("wayland") {
            Self::Wayland
        } else if display.is_some() {
            Self::X11
        } else {
            Self::None
        }
    }
}

pub(crate) fn display_server() -> DisplayServer {
    DisplayServer::detect(
        std::env::var_os("WAYLAND_DISPLAY")
            .as_deref()
            .and_then(|value| value.to_str()),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("DISPLAY")
            .as_deref()
            .and_then(|value| value.to_str()),
    )
}

pub(crate) fn apply_configured_policies(control: &Control, device: &crate::config::DeviceConfig) {
    let stable_ids: std::collections::HashSet<String> = control
        .enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|handle| !handle.identity_unstable())
        .map(|handle| handle.id().to_string())
        .collect();
    for (display_id, label) in &device.policy {
        if stable_ids.contains(display_id) {
            if let Some(policy) = crate::monitor::BrightnessPolicy::parse(label) {
                control.select(display_id, policy);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_server_detection_prefers_wayland_then_x11_then_none() {
        assert_eq!(DisplayServer::detect(None, None, None), DisplayServer::None);
        assert_eq!(
            DisplayServer::detect(Some("wayland-0"), Some("x11"), Some(":0")),
            DisplayServer::Wayland,
            "WAYLAND_DISPLAY wins over an x11 session type"
        );
        assert_eq!(
            DisplayServer::detect(None, Some("wayland"), Some(":0")),
            DisplayServer::Wayland
        );
        assert_eq!(
            DisplayServer::detect(None, Some("x11"), Some(":0")),
            DisplayServer::X11
        );
        assert_eq!(
            DisplayServer::detect(None, Some("tty"), None),
            DisplayServer::None
        );
    }

    #[test]
    fn stub_facade_selects_nothing_and_offers_no_lut() {
        let control: Control = Arc::new(StubControl);
        control.select("id-1", BrightnessPolicy::Gamma);
        assert!(control.gamma_backend().capture("card0-DP-1").is_none());
    }
}
