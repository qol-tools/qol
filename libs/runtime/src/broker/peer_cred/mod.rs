//! Peer-credential extraction for `UnixStream` connections.
//!
//! Two platform-specific paths are needed because the kernel APIs differ:
//!
//! - Linux exposes uid + pid + gid in one struct via `SO_PEERCRED`.
//! - macOS returns uid + group list via `LOCAL_PEERCRED` (xucred); pid
//!   is fetched separately via `LOCAL_PEEREPID`.
//!
//! See security plan card 06 for the rationale and primitives.

mod platform;

pub use platform::PeerStream;

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
    platform::current_uid() == Some(cred.uid)
}

/// Read peer credentials off a connected `UnixStream`.
///
/// On Linux this is one `getsockopt(SO_PEERCRED)` call; on macOS the
/// uid comes from `LOCAL_PEERCRED` and the pid from a second
/// `LOCAL_PEEREPID` call. The pid is treated as best-effort: a
/// platform that does not expose it returns `pid: None` rather than
/// failing the whole call.
pub fn peer_cred(stream: &PeerStream) -> std::io::Result<PeerCred> {
    platform::peer_cred(stream)
}
