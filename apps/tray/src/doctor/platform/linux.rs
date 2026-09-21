use std::path::Path;

use super::{DoctorPlatformOps, PlatformScope};

pub(crate) struct Platform;

impl DoctorPlatformOps for Platform {
    fn install_marker_required(&self, _current_exe: &Path) -> bool {
        true
    }

    fn matches_scope(&self, scope: PlatformScope) -> bool {
        matches!(scope, PlatformScope::Any | PlatformScope::Linux)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_linux_and_platform_independent_scopes() {
        let cases = [(PlatformScope::Any, true), (PlatformScope::Linux, true)];

        for (scope, expected) in cases {
            assert_eq!(Platform.matches_scope(scope), expected, "scope={scope:?}");
        }
    }
}
