use super::BuildPlatform;

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn executable_suffix(&self) -> &'static str {
        std::env::consts::EXE_SUFFIX
    }
}
