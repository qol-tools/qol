use std::io::{self, Cursor, Read};

use qol_conventions::operations::OperationKey;
use qol_runtime::local_ipc::MAX_MESSAGE_BYTES;

use super::*;
use crate::commands::peers::input::{parse, read_json, Action, Mutation};
use crate::commands::peers::pagination::Pages;

#[test]
fn convenience_parser_builds_canonical_actions() {
    let cases = [
        (vec!["status"], Action::Direct(Request::Status)),
        (vec!["list"], Action::Pages(Pages::Peers)),
        (vec!["grants", PEER], Action::Pages(Pages::Grants(peer()))),
        (vec!["tombstones"], Action::Pages(Pages::Tombstones)),
        (
            vec!["session", "name with spaces"],
            Action::Start(Request::StartSession {
                name: "name with spaces".into(),
            }),
        ),
        (
            vec!["create", "fixture"],
            Action::Start(Request::CreatePersistent {
                name: "fixture".into(),
            }),
        ),
        (vec!["open"], Action::Start(Request::OpenPersistent)),
        (vec!["stop"], Action::Mutation(Mutation::Stop)),
        (
            vec!["rename", "fixture"],
            Action::Mutation(Mutation::Rename("fixture".into())),
        ),
        (
            vec!["revoke", PEER],
            Action::Mutation(Mutation::Revoke(peer())),
        ),
        (
            vec!["set-grants", PEER],
            Action::Mutation(Mutation::SetGrants(peer(), Vec::new())),
        ),
        (vec!["request"], Action::Direct(Request::Status)),
    ];
    for (args, expected) in cases {
        let json = if args == ["request"] {
            r#"{"operation":"status"}"#
        } else {
            "[]"
        };
        assert_eq!(
            parse(&args, &mut json.as_bytes()).unwrap(),
            expected,
            "{args:?}"
        );
    }
}

#[test]
fn invalid_arguments_never_read_stdin_or_echo_payloads() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("invalid arguments must not read stdin")
        }
    }
    for args in [
        vec![],
        vec!["request", "secret-canary"],
        vec!["status", "secret-canary"],
        vec!["set-grants", "secret-canary"],
        vec!["grants", "secret-canary"],
        vec!["rename"],
        vec!["session"],
        vec!["create"],
        vec!["secret-canary"],
    ] {
        let error = parse(&args, &mut NoRead).unwrap_err().to_string();
        assert!(!error.contains("secret-canary"), "{args:?}: {error}");
    }
}

#[test]
fn canonical_request_parser_rejects_duplicate_unknown_and_trailing_input() {
    for json in [
        r#"{"operation":"secret-canary"}"#,
        r#"{"operation":"status","secret-canary":true}"#,
        r#"{"operation":"status","operation":"status"}"#,
        r#"{"operation":"status","\u006fperation":"status"}"#,
        r#"{"operation":"start_session","name":"secret-canary","name":"other"}"#,
        r#"{"operation":"status"} {"secret-canary":true}"#,
        r#"{"operation":"status"} secret-canary"#,
        r#"["secret-canary"]"#,
        "",
    ] {
        let error = read_json::<Request>(&mut json.as_bytes())
            .unwrap_err()
            .to_string();
        assert_eq!(
            error, "invalid canonical peer administration JSON on stdin",
            "{json}"
        );
    }
    let mut json = serde_json::to_string(&Request::Stop {
        expected: authority().expected(),
    })
    .unwrap();
    json = json.replace(
        "\"revision\":\"7\"",
        "\"revision\":\"7\",\"revision\":\"7\"",
    );
    assert!(read_json::<Request>(&mut json.as_bytes()).is_err());
}

#[test]
fn grants_use_the_canonical_operation_key_parser() {
    let valid =
        r#"[{"identity":{"scope":"stable","value":"fixture"},"kind":"query","name":"read"}]"#;
    let grants: Vec<OperationKey> = serde_json::from_str(valid).unwrap();
    assert_eq!(
        parse(&["set-grants", PEER], &mut valid.as_bytes()).unwrap(),
        Action::Mutation(Mutation::SetGrants(peer(), grants)),
    );
    for json in [
        valid.replace(
            "\"name\":\"read\"",
            "\"name\":\"read\",\"name\":\"secret-canary\"",
        ),
        valid.replace(
            "\"kind\":\"query\"",
            "\"kind\":\"query\",\"secret-canary\":true",
        ),
        valid.replace(
            "\"scope\":\"stable\"",
            "\"scope\":\"stable\",\"scope\":\"local\"",
        ),
        format!("{valid} []"),
    ] {
        let error = parse(&["set-grants", PEER], &mut json.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-canary"), "{error}");
    }
}

#[test]
fn stdin_is_bounded_before_read_completion_including_whitespace() {
    let prefix = br#"{"operation":"status"}"#;
    for length in [
        MAX_MESSAGE_BYTES - 1,
        MAX_MESSAGE_BYTES,
        MAX_MESSAGE_BYTES + 1,
    ] {
        let mut bytes = prefix.to_vec();
        bytes.resize(length, b' ');
        let mut input = Cursor::new(bytes);
        let result = read_json::<Request>(&mut input);
        assert_eq!(result.is_ok(), length <= MAX_MESSAGE_BYTES, "{length}");
        assert!(
            input.position() <= (MAX_MESSAGE_BYTES + 1) as u64,
            "{length}"
        );
    }
    let error = read_json::<Request>(&mut io::repeat(b' '))
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "peer administration stdin exceeds the local message boundary"
    );
}

#[test]
fn stdin_io_errors_do_not_echo_payloads() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("secret-canary"))
        }
    }
    let error = read_json::<Request>(&mut Broken).unwrap_err().to_string();
    assert_eq!(error, "could not read peer administration stdin");
}

#[test]
fn enrollment_stdin_is_bounded_redacted_and_never_accepted_as_an_argument() {
    use qol_peers::enrollment::MAX_INVITATION_BYTES;
    let action = parse(&["prepare"], &mut b"qol-link:synthetic-canary\n".as_slice()).unwrap();
    assert!(!format!("{action:?}").contains("synthetic-canary"));
    for args in [
        vec!["prepare", "synthetic-canary"],
        vec!["redeem", "synthetic-canary"],
        vec!["recover", "synthetic-canary", "127.0.0.1:1234"],
        vec!["invite", "synthetic-canary"],
        vec!["approve", "synthetic-canary", "synthetic-canary", PEER],
    ] {
        let error = parse(&args, &mut io::empty()).unwrap_err().to_string();
        assert!(!error.contains("synthetic-canary"), "{args:?}");
    }
    for length in [MAX_INVITATION_BYTES, MAX_INVITATION_BYTES + 1] {
        let value = format!("qol-link:{}", "x".repeat(length - 9));
        let mut input = Cursor::new(value.as_bytes());
        assert_eq!(
            parse(&["prepare"], &mut input).is_ok(),
            length <= MAX_INVITATION_BYTES,
            "{length}"
        );
    }
    let mut input = io::repeat(b'x');
    assert!(parse(&["prepare"], &mut input).is_err());
}

#[test]
fn pointz_commands_map_to_canonical_requests() {
    use qol_peers::admin::PointzRequest;
    let device: qol_peers::pointz::PointzDeviceId = "AQEBAQEBAQEBAQEBAQEBAQ".parse().unwrap();
    let pointz = |request| Action::Direct(Request::Pointz { request });
    let cases = [
        (vec!["pointz", "status"], pointz(PointzRequest::Status {})),
        (vec!["pointz", "devices"], Action::Pages(Pages::Phones)),
        (
            vec!["pointz", "pair"],
            pointz(PointzRequest::BeginPairing {}),
        ),
        (
            vec!["pointz", "cancel"],
            pointz(PointzRequest::CancelPairing {}),
        ),
        (
            vec!["pointz", "remove", "AQEBAQEBAQEBAQEBAQEBAQ"],
            Action::Mutation(Mutation::RemovePhone(device)),
        ),
    ];
    for (args, expected) in cases {
        assert_eq!(
            parse(&args, &mut io::empty()).unwrap(),
            expected,
            "{args:?}"
        );
    }
    assert!(parse(&["pointz", "remove", "short"], &mut io::empty()).is_err());
    assert!(parse(&["pointz"], &mut io::empty()).is_err());
}
