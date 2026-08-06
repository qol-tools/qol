use super::super::{DiscoveryError, WindowDiscovery, WindowInfo};
use crate::config::SwitchablePanelOverride;

pub struct Platform;

impl WindowDiscovery for Platform {
    fn visible_windows(
        &self,
        _include_minimized: bool,
        _switchable: &[SwitchablePanelOverride],
    ) -> Result<Vec<WindowInfo>, DiscoveryError> {
        Err(DiscoveryError)
    }
}
