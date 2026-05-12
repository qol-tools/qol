//! Structural invariants for the broker's AF_UNIX socket bind path.
//!
//! These tests pin the security-relevant wire properties of the broker
//! socket: the path is per-uid (no shared `/tmp/qol-runtime.sock` that any
//! local user could squat), the socket file is mode `0600`, and the parent
//! directory is mode `0700`. The bind helper refuses to create the socket
//! if the parent dir is world- or group-accessible.
//!
//! Refs: workspace/docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md
//! Refs: workspace/docs/superpowers/plans/2026-05-12-terminal-workspace-restore-security-plan.md (cards 06, 11)
//! Closes: RUNTIME-1.1.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use qol_runtime::broker::{self, BrokerPathError};

#[test]
fn broker_socket_path_is_per_uid() {
    // Two different uids must resolve to two different socket paths.
    // A shared path would let any same-machine user squat the file and
    // intercept connections at bind time.
    let a = broker::broker_socket_path_for_uid(1000, None);
    let b = broker::broker_socket_path_for_uid(1001, None);
    assert_ne!(
        a, b,
        "broker_socket_path_for_uid produced the same path for different uids: {a:?}. \
         The socket path must include the uid so two users on the same machine \
         do not collide on the same file."
    );
    let a_str = a.to_string_lossy();
    assert!(
        a_str.contains("1000"),
        "broker_socket_path_for_uid(1000) = {a:?} does not contain the uid; \
         uid-tagging is the structural property under test."
    );
}

#[test]
fn broker_socket_path_prefers_xdg_runtime_dir() {
    // When $XDG_RUNTIME_DIR is set and writable, the broker socket lives
    // beneath it (Linux convention; macOS lacks this var by default).
    // The fallback to /tmp/qol-runtime-<uid>.sock is exercised in the
    // None case in `broker_socket_path_is_per_uid` above.
    let with_xdg = broker::broker_socket_path_for_uid(1000, Some("/run/user/1000"));
    let s = with_xdg.to_string_lossy();
    assert!(
        s.starts_with("/run/user/1000/"),
        "broker_socket_path_for_uid did not honor XDG_RUNTIME_DIR; got {with_xdg:?}"
    );
    assert!(
        s.ends_with("broker.sock"),
        "broker socket filename is not `broker.sock`; got {with_xdg:?}"
    );
}

#[test]
fn broker_socket_path_fallback_is_tmp_with_uid() {
    // Without XDG_RUNTIME_DIR the broker falls back to /tmp/qol-runtime-<uid>.sock.
    // The fallback path must still be uid-scoped so two users cannot collide.
    let fallback = broker::broker_socket_path_for_uid(1000, None);
    let s = fallback.to_string_lossy();
    assert!(
        s.starts_with("/tmp/qol-runtime-"),
        "fallback broker path is not under /tmp/qol-runtime-<uid>.sock; got {fallback:?}"
    );
    assert!(
        s.contains("1000"),
        "fallback broker path is not uid-scoped; got {fallback:?}"
    );
}

#[test]
fn bind_broker_listener_creates_mode_0600_socket_with_0700_parent() {
    // The broker bind helper creates the socket file with mode 0600
    // (owner-only read/write) inside a parent dir at mode 0700.
    // Any other mode would let other local users read or connect.
    let tmp = tempdir();
    let parent = tmp.join("broker-dir");
    let sock = parent.join("broker.sock");

    broker::bind_broker_listener(&sock).expect("bind succeeds on fresh path");

    let parent_meta = std::fs::metadata(&parent).expect("parent dir created");
    let parent_mode = parent_meta.permissions().mode() & 0o777;
    assert_eq!(
        parent_mode, 0o700,
        "broker parent dir mode = {parent_mode:o}, expected 0700. \
         The dir must be inaccessible to other uids."
    );

    let sock_meta = std::fs::metadata(&sock).expect("socket file created");
    let sock_mode = sock_meta.permissions().mode() & 0o777;
    assert_eq!(
        sock_mode, 0o600,
        "broker socket file mode = {sock_mode:o}, expected 0600. \
         Connection attempts from other uids must be rejected at the \
         filesystem layer, not just by peer-cred checks."
    );
}

#[test]
fn bind_broker_listener_removes_stale_socket() {
    // If a previous daemon crashed leaving a socket file behind, bind
    // must unlink it before binding fresh. Otherwise the daemon refuses
    // to start.
    let tmp = tempdir();
    let parent = tmp.join("broker-dir");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    let sock = parent.join("broker.sock");
    // Pre-place a stale file at the socket path.
    std::fs::write(&sock, b"stale").unwrap();

    broker::bind_broker_listener(&sock).expect("bind succeeds even with stale socket file");
    let meta = std::fs::metadata(&sock).expect("socket file exists after bind");
    // It must be a socket, not the regular file we pre-placed.
    let ft = meta.file_type();
    use std::os::unix::fs::FileTypeExt;
    assert!(
        ft.is_socket(),
        "after bind, path is not a socket; stale-file cleanup did not happen"
    );
}

#[test]
fn bind_broker_listener_rejects_world_accessible_parent() {
    // Defense in depth: if the parent dir already exists with overly
    // permissive bits, refuse to bind rather than tighten silently.
    let tmp = tempdir();
    let parent = tmp.join("broker-dir");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();

    let sock = parent.join("broker.sock");
    let res = broker::bind_broker_listener(&sock);
    assert!(
        matches!(res, Err(BrokerPathError::ParentPermissive { .. })),
        "bind_broker_listener accepted a parent dir at mode 0755; got {res:?}. \
         The structural invariant is that the parent dir is locked to the owning uid."
    );
}

// Minimal local tempdir; avoids pulling in `tempfile` for one helper.
//
// AF_UNIX paths are bounded by `sun_path` (108 bytes Linux, 104 macOS),
// so keep the full directory name compact. We use the low 6 hex digits
// of the nanosecond timestamp plus the pid; collisions inside a single
// test run are vanishingly unlikely.
fn tempdir() -> std::path::PathBuf {
    let nonce: u128 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("qrt-{pid:x}-{:x}", nonce & 0xffffff));
    std::fs::create_dir_all(&p).unwrap();
    p
}
