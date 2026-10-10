use qol_host_fixes::residency::HostResidency;

use super::SessionPlatform;

pub struct Platform;

impl SessionPlatform for Platform {
    fn exit_restores_host(&self) -> bool {
        !HostResidency::current().is_resident()
    }
}
