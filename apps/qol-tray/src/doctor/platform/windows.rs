use std::path::Path;

use super::DoctorPlatformOps;

pub(crate) struct Platform;

impl DoctorPlatformOps for Platform {
    fn install_marker_required(&self, _current_exe: &Path) -> bool {
        true
    }
}
