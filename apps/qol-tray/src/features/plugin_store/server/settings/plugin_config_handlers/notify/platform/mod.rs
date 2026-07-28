#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::notify_plugin_reload;
#[cfg(unix)]
pub(super) use unix::notify_plugin_reload;
