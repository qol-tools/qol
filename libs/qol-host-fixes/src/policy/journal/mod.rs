mod platform;

#[cfg(target_os = "linux")]
pub(crate) use platform::recover_stage;
pub(crate) use platform::{read, remove_durable, write_durable};
