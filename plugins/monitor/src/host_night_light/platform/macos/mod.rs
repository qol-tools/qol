use std::path::Path;
use std::sync::Arc;

use super::super::{HostNightLight, UnavailableHostNightLight};

pub(crate) fn control(
    _config_root: Option<&Path>,
    _adoption_generation: Option<String>,
) -> Arc<dyn HostNightLight> {
    Arc::new(UnavailableHostNightLight(
        "Night Shift automation is unavailable; using CoreGraphics gamma",
    ))
}
