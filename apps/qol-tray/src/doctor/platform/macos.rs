use std::path::Path;

use super::DoctorPlatformOps;

pub(crate) struct Platform;

impl DoctorPlatformOps for Platform {
    fn install_marker_required(&self, current_exe: &Path) -> bool {
        !current_exe
            .to_string_lossy()
            .contains(".app/Contents/MacOS/")
    }
}
