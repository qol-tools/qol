use super::*;

#[test]
fn only_a_successful_matching_version_probe_replaces_the_installation() {
    let cases = [
        ("printf 'qol-tray 1.2.3\\n'", true, ""),
        ("printf 'qol-tray 1.2.4\\n'", false, "does not match"),
        ("printf 'qol-tray invalid\\n'", false, "Invalid staged"),
        ("printf 'another-app 1.2.3\\n'", false, "Unexpected staged"),
        ("exit 0", false, "Unexpected staged"),
        (
            "printf 'GLIBC_2.43 not found' >&2; exit 127",
            false,
            "GLIBC_2.43",
        ),
        ("printf 'qol-tray 1.2.3\\n'; exit 1", false, "exit status"),
        ("kill -TERM $$", false, "signal"),
        (
            "printf 'qol-tray 1.2.3'; head -c 16384 /dev/zero | tr '\\000' ' '",
            false,
            "does not match",
        ),
    ];
    for (body, accepted, reason) in cases {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download");
        let target = dir.path().join("installed");
        let content = format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 99\n[ \"$LD_BIND_NOW\" = 1 ] || exit 98\n{body}\n");
        std::fs::write(&source, &content).unwrap();
        std::fs::write(&target, b"working installation").unwrap();
        let staged = stage(&source, &target).unwrap();
        let result = promote(&staged, &target, Some("1.2.3"), PROBE_TIMEOUT);
        assert_eq!(result.is_ok(), accepted, "body={body}: {result:?}");
        let expected = if accepted {
            content.as_bytes()
        } else {
            b"working installation"
        };
        assert_eq!(std::fs::read(&target).unwrap(), expected, "body={body}");
        if let Err(error) = result {
            assert!(
                format!("{error:#}").contains(reason),
                "body={body}: {error:#}"
            );
            assert_eq!(
                crate::updates::plain_update_failure(&error),
                crate::updates::IncompatibleUpdate.to_string(),
                "body={body}"
            );
        }
        drop(staged);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "body={body}"
        );
    }
}

#[test]
fn unexecutable_and_hanging_candidates_preserve_the_installed_file() {
    for (content, reason, timeout) in [
        ("not an executable", "version check failed", PROBE_TIMEOUT),
        ("#!/missing/loader\n", "Could not execute", PROBE_TIMEOUT),
        (
            "#!/bin/sh\nexec sleep 30\n",
            "timed out",
            Duration::from_millis(50),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download");
        let target = dir.path().join("installed");
        std::fs::write(&source, content).unwrap();
        std::fs::write(&target, b"working installation").unwrap();
        let staged = stage(&source, &target).unwrap();
        let error = promote(&staged, &target, Some("1.2.3"), timeout).unwrap_err();
        assert!(
            format!("{error:#}").contains(reason),
            "content={content}: {error:#}"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"working installation",
            "content={content}"
        );
    }
}

#[test]
fn staging_is_private_and_probes_from_the_installation_directory() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("download");
    let target = dir.path().join("installed");
    let unrelated = target.with_extension("new");
    std::fs::write(&unrelated, b"unrelated").unwrap();
    std::fs::write(&source, "#!/bin/sh\n[ -f \"$(dirname \"$0\")/installed.new\" ] || exit 2\nprintf 'qol-tray 1.2.3\\n'\n").unwrap();
    let staged = stage(&source, &target).unwrap();
    assert_eq!(staged.parent(), target.parent());
    assert_ne!(staged.as_os_str(), unrelated.as_os_str());
    promote(&staged, &target, None, PROBE_TIMEOUT).unwrap();
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        std::fs::read(&source).unwrap()
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
#[ignore = "set QOL_TEST_UPDATE_BINARY and QOL_TEST_COMPATIBLE_BINARY to preserved releases"]
fn incompatible_release_cannot_replace_a_working_installation() {
    let source =
        std::env::var_os("QOL_TEST_UPDATE_BINARY").expect("QOL_TEST_UPDATE_BINARY required");
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("installed");
    std::fs::write(&target, b"working installation").unwrap();
    let staged = stage(Path::new(&source), &target).unwrap();
    let error = promote(&staged, &target, None, PROBE_TIMEOUT).unwrap_err();
    assert!(format!("{error:#}").contains("GLIBC_"), "{error:#}");
    assert_eq!(std::fs::read(&target).unwrap(), b"working installation");
    let compatible = std::env::var_os("QOL_TEST_COMPATIBLE_BINARY")
        .expect("QOL_TEST_COMPATIBLE_BINARY required");
    let staged = stage(Path::new(&compatible), &target).unwrap();
    promote(&staged, &target, None, PROBE_TIMEOUT).unwrap();
    assert_eq!(
        std::fs::read(&target).unwrap(),
        std::fs::read(&compatible).unwrap()
    );
}
