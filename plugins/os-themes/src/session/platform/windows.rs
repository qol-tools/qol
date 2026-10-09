use super::SessionPlatform;

pub struct Platform;

impl SessionPlatform for Platform {
    fn exit_restores_host(&self) -> bool {
        false
    }
}
