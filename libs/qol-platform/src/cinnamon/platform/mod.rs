#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(super) use linux::Session;
#[cfg(not(target_os = "linux"))]
pub(super) use unsupported::Session;
