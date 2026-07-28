use std::io;

use super::{PeerCred, PeerCredentialPlatform};

pub struct Platform;

impl PeerCredentialPlatform for Platform {
    type Stream = std::os::unix::net::UnixStream;

    fn current_uid() -> Option<u32> {
        Some(unsafe { libc::getuid() } as u32)
    }

    fn peer_cred(_stream: &Self::Stream) -> io::Result<PeerCred> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "peer credentials are unavailable on this platform",
        ))
    }
}
