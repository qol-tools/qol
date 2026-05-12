//! Peer-credential extraction for `UnixStream` connections.
//!
//! Two platform-specific paths are needed because the kernel APIs differ:
//!
//! - Linux exposes uid + pid + gid in one struct via `SO_PEERCRED`.
//! - macOS returns uid + group list via `LOCAL_PEERCRED` (xucred); pid
//!   is fetched separately via `LOCAL_PEEREPID`.
//!
//! See security plan card 06 for the rationale and primitives.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

/// Identity of the peer process attached to a connected `UnixStream`.
///
/// `uid` is the structural-authn field: the broker rejects any peer
/// whose `uid` differs from the daemon's own. `pid` is provided where
/// the kernel exposes it and is used by higher layers to look the peer
/// up in the supervised-plugin registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerCred {
    pub uid: u32,
    pub pid: Option<u32>,
}

/// Returns true when `cred.uid` matches the current process uid.
///
/// This is the post-accept gate: a peer running as a different uid is
/// closed without reading any request bytes.
pub fn is_same_uid(cred: &PeerCred) -> bool {
    // SAFETY: getuid() always succeeds and is signal-safe.
    let self_uid = unsafe { libc::getuid() } as u32;
    cred.uid == self_uid
}

/// Read peer credentials off a connected `UnixStream`.
///
/// On Linux this is one `getsockopt(SO_PEERCRED)` call; on macOS the
/// uid comes from `LOCAL_PEERCRED` and the pid from a second
/// `LOCAL_PEEREPID` call. The pid is treated as best-effort: a
/// platform that does not expose it returns `pid: None` rather than
/// failing the whole call.
#[cfg(target_os = "linux")]
pub fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
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
    // SAFETY: fd is a valid socket fd owned by `stream`; buffer is
    // sized and aligned via `std::mem::size_of`.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCred {
        uid: cred.uid,
        pid: Some(cred.pid as u32),
    })
}

#[cfg(target_os = "macos")]
pub fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
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

    let fd = stream.as_raw_fd();
    let mut cred: Xucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<Xucred>() as libc::socklen_t;
    // SAFETY: see linux path; same arguments shape, different option.
    let r = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    if cred.cr_version != XUCRED_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected xucred version {}", cred.cr_version),
        ));
    }

    // LOCAL_PEEREPID is best-effort: if it fails we still know the uid.
    let mut pid: libc::pid_t = 0;
    let mut plen = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: same fd, same shape of arguments as above.
    let pid_r = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEEREPID,
            &mut pid as *mut _ as *mut libc::c_void,
            &mut plen,
        )
    };
    let pid_opt = if pid_r == 0 && pid > 0 {
        Some(pid as u32)
    } else {
        None
    };
    Ok(PeerCred {
        uid: cred.cr_uid,
        pid: pid_opt,
    })
}

/// Fallback for non-Linux, non-macOS unix targets (e.g. *BSD): treat
/// peer-cred extraction as unimplemented. The build is gated to unix
/// already, so this branch only fires on a niche target where the
/// daemon would refuse to bind anyway.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn peer_cred(_stream: &UnixStream) -> io::Result<PeerCred> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "peer_cred is only implemented for linux and macos",
    ))
}
