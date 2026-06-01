//! Structural invariants for peer-credential extraction on the broker
//! socket.
//!
//! The broker authorizes a connection by reading the peer's uid (and
//! pid where the kernel exposes it) from the connected stream. This
//! lifts authorization from "anyone who can reach the socket" to
//! "only processes running as the same uid as the daemon", the
//! structural collapse that replaces bearer-token-only auth.
//!
//! Refs: workspace/docs/superpowers/plans/2026-05-12-terminal-workspace-restore-security-plan.md (card 06)
//! Closes: RUNTIME-1.4.

#![cfg(unix)]

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;

use qol_runtime::broker::{self, PeerCred};

#[test]
fn peer_cred_exposes_uid_field() {
    // The PeerCred struct must expose `uid` as a `u32`. The caller
    // uses uid to reject foreign-uid connections before any request
    // byte is read.
    let cred = PeerCred {
        uid: 1000,
        pid: Some(42),
    };
    assert_eq!(cred.uid, 1000);
}

#[test]
fn peer_cred_pid_is_optional() {
    // Linux's SO_PEERCRED returns a pid; macOS's LOCAL_PEEREPID returns
    // one separately. The shape is Option<u32> so platforms without a
    // pid surface (or where the second getsockopt fails) can still
    // return a PeerCred with uid alone.
    let cred = PeerCred {
        uid: 1000,
        pid: None,
    };
    assert_eq!(cred.uid, 1000);
    assert!(cred.pid.is_none());
}

#[test]
fn peer_cred_round_trip_via_socketpair_reports_current_uid() {
    // Live socketpair: connect a client to a listener bound at a temp
    // path, accept on the server side, and assert peer_cred() reports
    // the current process's uid.
    let sock = temp_sock_path();
    let listener = UnixListener::bind(&sock).expect("bind temp listener");

    let client_thread = {
        let path = sock.clone();
        thread::spawn(move || {
            let mut s = UnixStream::connect(&path).expect("client connect");
            // Hold the socket open until the server has read peer cred.
            let mut buf = [0u8; 1];
            let _ = s.read(&mut buf);
        })
    };

    let (server_stream, _addr) = listener.accept().expect("server accept");
    let cred = broker::peer_cred(&server_stream).expect("peer_cred succeeds on local stream");

    let self_uid = current_uid();
    assert_eq!(
        cred.uid, self_uid,
        "peer_cred reported uid={}, expected current uid={}; \
         peer-cred extraction is the structural authn layer and must \
         agree with getuid() for loopback connections from this process",
        cred.uid, self_uid
    );

    // Tear down: dropping `server_stream` and the listener wakes the
    // client read so its thread can exit cleanly.
    drop(server_stream);
    drop(listener);
    let _ = client_thread.join();
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn is_same_uid_accepts_self_and_rejects_others() {
    // The check the broker runs after extracting peer_cred. Same-uid
    // returns true; foreign uid returns false. This is a pure function
    // so the test does not need a live socket.
    let self_uid = current_uid();
    assert!(
        broker::is_same_uid(&PeerCred {
            uid: self_uid,
            pid: None,
        }),
        "is_same_uid rejected the daemon's own uid; \
         this would lock the broker out of its own socket"
    );
    let other_uid = self_uid.wrapping_add(1);
    assert!(
        !broker::is_same_uid(&PeerCred {
            uid: other_uid,
            pid: None,
        }),
        "is_same_uid accepted a foreign uid ({other_uid}); \
         the structural authorization invariant is broken"
    );
}

fn temp_sock_path() -> PathBuf {
    let nonce: u128 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("qrt-pc-{pid:x}-{:x}.sock", nonce & 0xffffff));
    let _ = std::fs::remove_file(&p);
    p
}

fn current_uid() -> u32 {
    // SAFETY: getuid() always succeeds.
    unsafe { libc::getuid() as u32 }
}
