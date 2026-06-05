#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod non_macos;

#[cfg(target_os = "macos")]
pub(crate) use macos::run;
#[cfg(not(target_os = "macos"))]
pub(crate) use non_macos::run;
