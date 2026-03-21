#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

pub(super) fn notify_plugin_reload(socket_path: &str) -> bool {
    #[cfg(unix)]
    return unix::notify_plugin_reload(socket_path);

    #[cfg(not(unix))]
    fallback::notify_plugin_reload(socket_path)
}
