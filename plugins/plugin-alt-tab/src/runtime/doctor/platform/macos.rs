use super::Inspection;

pub(in crate::runtime::doctor) fn inspect() -> Inspection {
    Inspection {
        platform: "macos",
        supported: true,
        backend: "appkit-windowserver",
        display_ready: true,
        display_env_set: false,
        wayland_env_set: false,
        session_type: None,
    }
}
