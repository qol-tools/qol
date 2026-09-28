use super::*;

#[test]
fn local_admin_contract_rejects_unknown_fields_tags_and_noncanonical_revisions() {
    for wire in [
        r#"{"operation":"start_session","name":"local","root":"/untrusted"}"#,
        r#"{"operation":"rename","expected":"01","name":"local"}"#,
        r#"{"operation":"rename","expected":1,"name":"local"}"#,
        r#"{"operation":"rename","expected":"1","name":"a","name":"b"}"#,
        r#"{"operation":"insert_link"}"#,
        r#"{"operation":"open_persistent","root":"/untrusted"}"#,
    ] {
        assert!(serde_json::from_str::<Request>(wire).is_err(), "{wire}");
    }
    for wire in [
        r#"{"code":"storage","path":"/private"}"#,
        r#"{"code":"stale_revision","expected":"0","current":"1","payload":"private"}"#,
        r#"{"code":"unknown"}"#,
    ] {
        assert!(
            serde_json::from_str::<AuthorityError>(wire).is_err(),
            "{wire}"
        );
    }
}

#[test]
fn authority_errors_have_one_strict_sanitized_wire_representation() {
    let errors = [
        AuthorityError::Faulted,
        AuthorityError::StaleRevision {
            expected: StoreRevision::INITIAL,
            current: StoreRevision::new(1),
        },
        AuthorityError::RevisionExhausted,
        AuthorityError::Capacity,
        AuthorityError::InvalidName,
        AuthorityError::InvalidGrant,
        AuthorityError::DuplicateGrant,
        AuthorityError::DuplicatePeer,
        AuthorityError::UnknownPeer,
        AuthorityError::Revoked,
        AuthorityError::LocalPeer,
        AuthorityError::AlreadyExists,
        AuthorityError::MissingStore,
        AuthorityError::WriterBusy,
        AuthorityError::UnsafeStore,
        AuthorityError::InvalidSnapshot,
        AuthorityError::UnsupportedVersion,
        AuthorityError::Identity,
        AuthorityError::UnsupportedPlatform,
        AuthorityError::Storage,
        AuthorityError::Transport,
    ];
    for error in errors {
        let response = Response::Error {
            error: error.into(),
        };
        let wire = serde_json::to_vec(&response).unwrap();
        assert!(wire.len() < 512, "{error:?}");
        assert_eq!(serde_json::from_slice::<Response>(&wire).unwrap(), response);
    }
    assert_eq!(
        serde_json::to_value(AuthorityError::WriterBusy).unwrap(),
        serde_json::json!({"code": "writer_busy"})
    );
}

#[cfg(feature = "service")]
#[test]
fn service_error_facade_preserves_the_canonical_type() {
    let error: crate::service::AuthorityError = crate::AuthorityError::WriterBusy;
    let canonical: crate::AuthorityError = error;
    assert_eq!(canonical, AuthorityError::WriterBusy);
}
