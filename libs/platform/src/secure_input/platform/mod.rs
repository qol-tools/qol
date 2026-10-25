use super::SecureInputHolder;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub(super) use macos::SecureInput;
#[cfg(not(target_os = "macos"))]
pub(super) use unsupported::SecureInput;

pub(super) trait SecureInputApi {
    fn enabled(&self) -> bool;
    fn holder(&self) -> Option<SecureInputHolder>;
}
