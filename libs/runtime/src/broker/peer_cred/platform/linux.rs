use std::io;
use std::os::fd::AsRawFd;

use super::{PeerCred, PeerCredentialPlatform};

pub struct Platform;

impl PeerCredentialPlatform for Platform {
    type Stream = std::os::unix::net::UnixStream;

    fn current_uid() -> Option<u32> {
        Some(unsafe { libc::getuid() } as u32)
    }

    fn peer_cred(stream: &Self::Stream) -> io::Result<PeerCred> {
        #[repr(C)]
        struct Ucred {
            pid: libc::pid_t,
            uid: libc::uid_t,
            gid: libc::gid_t,
        }

        let fd = stream.as_raw_fd();
        let mut cred = Ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<Ucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PeerCred {
            uid: cred.uid,
            pid: Some(cred.pid as u32),
        })
    }
}
