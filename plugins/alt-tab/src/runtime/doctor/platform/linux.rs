use super::Inspection;

pub(in crate::runtime::doctor) fn inspect() -> Inspection {
    Inspection {
        platform: "linux",
        supported: true,
        backend: "x11",
        display_ready: env_is_set("DISPLAY"),
        display_env_set: env_is_set("DISPLAY"),
        wayland_env_set: env_is_set("WAYLAND_DISPLAY"),
        session_type: session_type(),
    }
}

fn env_is_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn session_type() -> Option<String> {
    std::env::var("XDG_SESSION_TYPE")
        .ok()
        .filter(|value| !value.trim().is_empty())
}
