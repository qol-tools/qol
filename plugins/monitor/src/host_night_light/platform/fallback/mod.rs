use std::path::Path;
use std::sync::Arc;

use super::super::{HostNightLight, UnavailableHostNightLight};

pub(crate) fn control(_config_root: Option<&Path>) -> Arc<dyn HostNightLight> {
    Arc::new(UnavailableHostNightLight(
        "Native night light and display gamma control are unavailable on this platform",
    ))
}
