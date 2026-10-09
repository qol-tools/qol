#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::{
    console_launch, console_probe, console_send, console_title, spawn_backend, system_backends,
};
#[cfg(windows)]
pub(crate) use windows::{
    console_launch, console_probe, console_send, console_title, spawn_backend, system_backends,
};
