//! Stub platform implementation for unsupported OSes.
//!
//! `open_settings` is fire-and-forget on real platforms; on unsupported OSes
//! we log a warning and return rather than panic so the rest of the binary
//! continues to function (discovery, status server, etc.).

pub fn open_settings() {
    log::warn!(
        "{}: open_settings is not implemented on this OS; visit {} manually",
        super::PLUGIN_ID,
        super::settings_url()
    );
}
