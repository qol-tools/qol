#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(in crate::service::authority) use unix::Store;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(in crate::service::authority) use unsupported::Store;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[derive(Clone, Copy)]
pub(in crate::service::authority) enum CommitFault {
    BeforeReplace,
    AfterReplace,
}
