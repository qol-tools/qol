#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(crate) use linux::CHECKS;
#[cfg(not(target_os = "linux"))]
pub(crate) use unsupported::CHECKS;
