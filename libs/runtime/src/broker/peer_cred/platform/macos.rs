use std::io;
use std::os::fd::AsRawFd;

use super::{PeerCred, PeerCredentialPlatform};

const SOL_LOCAL: libc::c_int = 0;
const LOCAL_PEERCRED: libc::c_int = 0x001;
const LOCAL_PEEREPID: libc::c_int = 0x003;
const XUCRED_VERSION: libc::c_uint = 0;
const NGROUPS: usize = 16;

#[repr(C)]
struct Xucred {
    cr_version: libc::c_uint,
    cr_uid: libc::uid_t,
    cr_ngroups: libc::c_short,
    cr_groups: [libc::gid_t; NGROUPS],
}

pub struct Platform;

impl PeerCredentialPlatform for Platform {
    type Stream = std::os::unix::net::UnixStream;

    fn current_uid() -> Option<u32> {
        Some(unsafe { libc::getuid() } as u32)
    }

    fn peer_cred(stream: &Self::Stream) -> io::Result<PeerCred> {
        let fd = stream.as_raw_fd();
        let mut cred: Xucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<Xucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if cred.cr_version != XUCRED_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected xucred version {}", cred.cr_version),
            ));
        }

        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let pid_result = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEEREPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut length,
            )
        };
        let pid = (pid_result == 0 && pid > 0).then_some(pid as u32);
        Ok(PeerCred {
            uid: cred.cr_uid,
            pid,
        })
    }
}
