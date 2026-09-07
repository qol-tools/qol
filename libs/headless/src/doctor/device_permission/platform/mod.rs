#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(super) use linux::LinuxI2cProbe as PlatformI2cProbe;
#[cfg(not(target_os = "linux"))]
pub(super) use unsupported::UnsupportedI2cProbe as PlatformI2cProbe;
