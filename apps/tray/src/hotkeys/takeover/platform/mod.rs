#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{
    compositor, dump, get_schema_value, list_schema, read, read_spice, reset, spice_configs, write,
    write_spice,
};
#[cfg(target_os = "linux")]
pub(super) use linux::{
    compositor, dump, get_schema_value, list_schema, read, read_spice, reset, spice_configs, write,
    write_spice,
};
#[cfg(target_os = "macos")]
pub(super) use macos::{
    compositor, dump, get_schema_value, list_schema, read, read_spice, reset, spice_configs, write,
    write_spice,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    compositor, dump, get_schema_value, list_schema, read, read_spice, reset, spice_configs, write,
    write_spice,
};
