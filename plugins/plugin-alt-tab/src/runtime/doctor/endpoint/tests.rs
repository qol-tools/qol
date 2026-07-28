use qol_headless::DoctorStatus;

use super::*;

#[test]
fn missing_endpoint_inspection_never_creates_or_connects() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.sock");

    let result = result(&missing);

    assert_eq!(result.status, DoctorStatus::Warn);
    assert!(!missing.exists());
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["connected"].as_bool()),
        Some(false)
    );
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["created"].as_bool()),
        Some(false)
    );
}

#[test]
fn regular_file_is_not_mistaken_for_a_daemon_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("endpoint");
    std::fs::write(&file, b"not a socket").unwrap();

    let result = result(&file);

    assert_eq!(result.status, DoctorStatus::Fail);
}

#[cfg(unix)]
#[test]
fn endpoint_inspection_does_not_connect_to_a_listening_socket() {
    use std::os::unix::net::UnixListener;

    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();

    let result = result(&socket);
    let error = listener.accept().unwrap_err();

    assert_eq!(result.status, DoctorStatus::Ok);
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
}
