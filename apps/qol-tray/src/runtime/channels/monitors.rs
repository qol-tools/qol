use std::time::Duration;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct MonitorsChannel {
    platform: SharedPlatform,
    monitors: Vec<qol_runtime::MonitorBounds>,
}

impl MonitorsChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        let monitors = platform.physical_monitors();
        Self { platform, monitors }
    }

    pub(crate) fn monitors(&self) -> &[qol_runtime::MonitorBounds] {
        &self.monitors
    }
}

impl Channel for MonitorsChannel {
    fn poll(&mut self) -> bool {
        let fresh = self.platform.physical_monitors();
        if !fresh.is_empty() && fresh != self.monitors {
            self.monitors = fresh;
            return true;
        }
        false
    }

    fn min_interval(&self) -> Duration {
        REFRESH_INTERVAL
    }
}
