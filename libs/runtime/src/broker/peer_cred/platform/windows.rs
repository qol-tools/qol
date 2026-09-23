use std::io;

use super::{PeerCred, PeerCredentialPlatform};

pub struct PeerStream;

pub struct Platform;

impl PeerCredentialPlatform for Platform {
    type Stream = PeerStream;

    fn current_uid() -> Option<u32> {
        None
    }

    fn peer_cred(_stream: &Self::Stream) -> io::Result<PeerCred> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "peer credentials are unavailable on Windows",
        ))
    }
}
