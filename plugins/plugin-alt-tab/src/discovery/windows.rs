use super::{DiscoveryError, WindowDiscovery, WindowInfo};

pub struct Platform;

impl WindowDiscovery for Platform {
    fn visible_windows(&self, _include_minimized: bool) -> Result<Vec<WindowInfo>, DiscoveryError> {
        Err(DiscoveryError::Unsupported)
    }
}
