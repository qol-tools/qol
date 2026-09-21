#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod verified;

#[cfg(target_os = "macos")]
pub(in crate::features::task_runner) use macos::CommandTree;
#[cfg(not(target_os = "macos"))]
pub(in crate::features::task_runner) use verified::CommandTree;
