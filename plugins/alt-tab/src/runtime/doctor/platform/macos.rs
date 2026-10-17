use super::Inspection;

pub(in crate::runtime::doctor) fn inspect() -> Inspection {
    Inspection {
        platform: qol_platform::Os::Macos.name(),
        supported: true,
        backend: "appkit-windowserver",
        display_ready: true,
        display_env_set: false,
        wayland_env_set: false,
        session_type: None,
    }
}
