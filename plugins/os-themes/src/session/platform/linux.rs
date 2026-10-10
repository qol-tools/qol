use std::path::PathBuf;

use qol_host_fixes::residency::HostResidency;

use super::SessionPlatform;

pub struct Platform;

impl SessionPlatform for Platform {
    fn exit_restores_host(&self) -> bool {
        !HostResidency::current().is_resident()
    }
}

impl Platform {
    pub(crate) fn session_subdir(subdir: &str) -> PathBuf {
        super::store::session_subdir(subdir)
    }
}
