use super::Inspection;

pub(in crate::runtime::doctor) fn inspect() -> Inspection {
    Inspection {
        platform: std::env::consts::OS,
        supported: false,
        backend: "unsupported",
        display_ready: false,
        display_env_set: false,
        wayland_env_set: false,
        session_type: None,
    }
}
