#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::{read, recover_stage, remove_durable, write_durable};

#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(not(target_os = "linux"))]
pub(crate) use fallback::{read, remove_durable, write_durable};
