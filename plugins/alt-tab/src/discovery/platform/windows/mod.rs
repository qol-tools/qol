use super::super::{DiscoveryError, WindowDiscovery, WindowInfo};
use crate::config::SwitchablePanels;

pub struct Platform;

impl WindowDiscovery for Platform {
    fn visible_windows(
        &self,
        _include_minimized: bool,
        _switchable: &SwitchablePanels,
    ) -> Result<Vec<WindowInfo>, DiscoveryError> {
        Err(DiscoveryError)
    }
}
