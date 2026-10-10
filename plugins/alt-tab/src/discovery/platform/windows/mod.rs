use qol_windowing::platform::windows::{top_level_windows, Window};

use super::super::{DiscoveryError, WindowDiscovery, WindowInfo};
use crate::config::SwitchablePanelOverride;

pub struct Platform;

impl WindowDiscovery for Platform {
    fn visible_windows(
        &self,
        include_minimized: bool,
        _switchable: &[SwitchablePanelOverride],
    ) -> Result<Vec<WindowInfo>, DiscoveryError> {
        let own_pid = std::process::id();
        Ok(top_level_windows()
            .into_iter()
            .filter(|window| window.is_switchable() && window.pid() != Some(own_pid))
            .filter(|window| include_minimized || !window.is_minimized())
            .filter_map(window_info)
            .collect())
    }
}

fn window_info(window: Window) -> Option<WindowInfo> {
    let title = window.title();
    if title.is_empty() {
        return None;
    }
    let frame = window.frame()?;
    Some(WindowInfo {
        id: window.id().as_u32()?,
        title,
        app_name: app_name(window),
        icon: None,
        width: frame.width as f32,
        height: frame.height as f32,
        is_minimized: window.is_minimized(),
    })
}

fn app_name(window: Window) -> String {
    window
        .pid()
        .and_then(|pid| qol_process::process_image_path(pid).ok())
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}
