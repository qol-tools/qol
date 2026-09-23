#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
mod fallback_unix;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub use fallback::Platform;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
pub use fallback_unix::Platform;
#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;

use super::PeerCred;

pub trait PeerCredentialPlatform {
    type Stream;

    fn current_uid() -> Option<u32>;
    fn peer_cred(stream: &Self::Stream) -> std::io::Result<PeerCred>;
}

pub type PeerStream = <Platform as PeerCredentialPlatform>::Stream;

pub(super) fn current_uid() -> Option<u32> {
    Platform::current_uid()
}

pub(super) fn peer_cred(stream: &PeerStream) -> std::io::Result<super::PeerCred> {
    Platform::peer_cred(stream)
}
