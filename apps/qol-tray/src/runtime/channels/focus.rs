use std::time::Duration;

use qol_runtime::MonitorBounds;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const MIN_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FocusChannel {
    platform: SharedPlatform,
    bounds: Option<MonitorBounds>,
    poll_allowed: bool,
}

impl FocusChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        let poll_allowed = platform.poll_focused_window();
        Self {
            platform,
            bounds: None,
            poll_allowed,
        }
    }

    pub(crate) fn bounds(&self) -> Option<MonitorBounds> {
        self.bounds
    }
}

impl Channel for FocusChannel {
    fn poll(&mut self) -> bool {
        if !self.poll_allowed {
            return false;
        }
        let fresh = self.platform.focused_window_bounds();
        if fresh.is_some() && fresh != self.bounds {
            log::debug!(
                "[runtime/focus_ch] CHANGED old={:?} new={:?}",
                self.bounds.map(|b| (b.x, b.y, b.width, b.height)),
                fresh.map(|b| (b.x, b.y, b.width, b.height))
            );
            self.bounds = fresh;
            return true;
        }
        false
    }

    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}
