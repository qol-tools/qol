#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(in crate::service::authority) use linux::Store;
#[cfg(not(target_os = "linux"))]
pub(in crate::service::authority) use unsupported::Store;

#[cfg(all(test, target_os = "linux"))]
#[derive(Clone, Copy)]
pub(in crate::service::authority) enum CommitFault {
    BeforeReplace,
    AfterReplace,
}
