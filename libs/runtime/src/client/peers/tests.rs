use std::sync::{Arc, Mutex};

use super::*;

struct ScriptedConnection {
    written: Arc<Mutex<Vec<u8>>>,
    response: io::Cursor<Vec<u8>>,
    read_error: Option<io::ErrorKind>,
}

impl io::Read for ScriptedConnection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.read_error {
            return Err(io::Error::from(kind));
        }
        io::Read::read(&mut self.response, bytes)
    }
}

impl io::Write for ScriptedConnection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.written.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl platform::Connection for ScriptedConnection {
    fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    fn try_clone(&self) -> io::Result<Box<dyn platform::Connection>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    fn shutdown(&self, _: std::net::Shutdown) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn mutation_timeout_disconnect_and_invalid_replies_never_replay() {
    for (response, read_error, query_error) in [
        (
            vec![],
            Some(io::ErrorKind::TimedOut),
            PeerAdminClientError::Timeout,
        ),
        (
            vec![],
            Some(io::ErrorKind::WouldBlock),
            PeerAdminClientError::Timeout,
        ),
        (vec![], None, PeerAdminClientError::Disconnected),
        (
            b"not json\n".to_vec(),
            None,
            PeerAdminClientError::InvalidReply,
        ),
        (
            b"{\"result\":\"error\",\"error\":{\"code\":\"inactive\",\"unexpected\":null}}\n".to_vec(),
            None,
            PeerAdminClientError::InvalidReply,
        ),
        (
            b"{\"result\":\"error\",\"error\":{\"code\":\"authority\",\"error\":{\"code\":\"storage\",\"code\":\"storage\"}}}\n".to_vec(),
            None,
            PeerAdminClientError::InvalidReply,
        ),
        (
            vec![b'x'; MAX_MESSAGE_BYTES + 1],
            None,
            PeerAdminClientError::InvalidReply,
        ),
    ] {
        let expected = qol_peers::admin::ExpectedAuthority {
            authority_id: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
            activation_id: qol_peers::admin::ActivationId::from_bytes([1; 16]),
            revision: qol_peers::StoreRevision::INITIAL,
        };
        for request in [
            Request::Status,
            Request::Network,
            Request::OpenPersistent,
            Request::Rename { expected, name: "local".into() },
            Request::SetGrants { expected, peer_id: expected.authority_id, grants: vec![] },
            Request::Revoke { expected, peer_id: expected.authority_id },
            Request::Stop { expected },
        ] {
            let mutation = request.is_mutation();
            let written = Arc::new(Mutex::new(Vec::new()));
            let connection = ScriptedConnection {
                written: written.clone(),
                response: io::Cursor::new(response.clone()),
                read_error,
            };
            let payload = encode(request).unwrap();
            let result = exchange(Box::new(connection), &payload, mutation, ADMIN_TIMEOUT);
            let expected = if mutation {
                PeerAdminClientError::OutcomeUnknown
            } else {
                query_error
            };
            assert_eq!(
                result,
                Err(expected),
                "mutation={mutation} error={read_error:?}"
            );
            assert_eq!(
                *written.lock().unwrap(),
                *payload,
                "request must be delivered only once"
            );
        }
    }
}

#[test]
fn request_bounds_include_the_runtime_envelope_and_newline() {
    let baseline = encode(Request::StartSession {
        name: String::new(),
    })
    .unwrap()
    .len();
    let payload = encode(Request::StartSession {
        name: "x".repeat(MAX_MESSAGE_BYTES - baseline),
    })
    .unwrap();
    assert_eq!(payload.len(), MAX_MESSAGE_BYTES);
    assert_eq!(
        encode(Request::StartSession {
            name: "x".repeat(MAX_MESSAGE_BYTES - baseline + 1)
        }),
        Err(PeerAdminClientError::RequestTooLarge)
    );
    let grant = qol_conventions::operations::OperationKey::new(
        qol_conventions::plugin_id::PluginUid::new("\\\"".repeat(128)),
        qol_conventions::operations::OperationKind::Query,
        "a".repeat(256),
    );
    let request = Request::SetGrants {
        expected: qol_peers::admin::ExpectedAuthority {
            authority_id: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse()
                .unwrap(),
            activation_id: qol_peers::admin::ActivationId::from_bytes([1; 16]),
            revision: qol_peers::StoreRevision::INITIAL,
        },
        peer_id: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            .parse()
            .unwrap(),
        grants: (0..128)
            .map(|index| {
                let mut grant = grant.clone();
                grant.name = format!("q{index:03}{}", "a".repeat(252));
                grant
            })
            .collect(),
    };
    let client = PlatformStateClient::new(std::path::PathBuf::from("missing-test-socket"));
    assert_eq!(
        client.peer_admin(request),
        Err(PeerAdminClientError::RequestTooLarge)
    );
}

#[test]
fn bounded_typed_authority_errors_are_known_outcomes() {
    let response = Response::Error {
        error: qol_peers::AuthorityError::WriterBusy.into(),
    };
    let mut wire = serde_json::to_vec(&response).unwrap();
    wire.push(b'\n');
    let connection = ScriptedConnection {
        written: Arc::default(),
        response: io::Cursor::new(wire),
        read_error: None,
    };
    let payload = encode(Request::OpenPersistent).unwrap();
    assert_eq!(
        exchange(Box::new(connection), &payload, true, ADMIN_TIMEOUT),
        Ok(response)
    );
}

#[test]
fn connection_failures_preserve_unsupported_and_credential_rejection() {
    for (kind, expected) in [
        (
            io::ErrorKind::Unsupported,
            PeerAdminClientError::UnsupportedPlatform,
        ),
        (
            io::ErrorKind::PermissionDenied,
            PeerAdminClientError::Unauthorized,
        ),
        (io::ErrorKind::NotFound, PeerAdminClientError::Unavailable),
    ] {
        assert_eq!(connection_error(io::Error::from(kind)), expected);
    }
}

#[cfg(not(unix))]
#[test]
fn unsupported_transport_cannot_create_a_local_authority() {
    let client = PlatformStateClient::new(std::path::PathBuf::from("unused"));
    assert_eq!(
        client.peer_admin(Request::StartSession {
            name: "local".into()
        }),
        Err(PeerAdminClientError::UnsupportedPlatform)
    );
}

#[test]
fn additive_network_queries_and_stopping_replies_use_the_canonical_client_contract() {
    use qol_peers::admin::{
        ActivationId, Lifecycle, NetworkRevision, NetworkSummary, SessionCursor, Status,
    };
    use qol_peers::{network::NetworkStatus, StoreRevision};
    let authority_id = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        .parse()
        .unwrap();
    let activation_id = ActivationId::from_bytes([0; 16]);
    let expected = qol_peers::admin::ExpectedAuthority {
        authority_id,
        activation_id,
        revision: StoreRevision::INITIAL,
    };
    let cursor = SessionCursor {
        authority_id,
        activation_id,
        revision: StoreRevision::INITIAL,
        network_revision: NetworkRevision::default(),
        offset: 0,
    };
    for (request, response) in [
        (
            Request::Network,
            Response::Network {
                network: NetworkSummary {
                    authority: expected,
                    network_revision: NetworkRevision::default(),
                    status: NetworkStatus::default(),
                },
            },
        ),
        (
            Request::Sessions { cursor },
            Response::Sessions {
                page: qol_peers::admin::SessionPage {
                    cursor,
                    total: 0,
                    items: Vec::new(),
                    next: None,
                },
            },
        ),
        (
            Request::Stop { expected },
            Response::Status {
                status: Status {
                    lifecycle: Lifecycle::Stopping,
                    authority: None,
                },
            },
        ),
    ] {
        let mut wire = serde_json::to_vec(&response).unwrap();
        wire.push(b'\n');
        let payload = encode(request.clone()).unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        let connection = ScriptedConnection {
            written: written.clone(),
            response: io::Cursor::new(wire),
            read_error: None,
        };
        let actual = exchange(
            Box::new(connection),
            &payload,
            request.is_mutation(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(actual, response, "{request:?}");
        assert_eq!(*written.lock().unwrap(), *payload, "{request:?}");
    }
}

#[test]
fn enrollment_unknown_delivery_uses_the_same_single_dispatch_boundary() {
    use qol_peers::{
        admin::{ActivationId, EnrollmentRequest, ExpectedAuthority},
        enrollment::ExportedInvitation,
    };
    let expected = ExpectedAuthority {
        authority_id: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            .parse()
            .unwrap(),
        activation_id: ActivationId::from_bytes([1; 16]),
        revision: qol_peers::StoreRevision::INITIAL,
    };
    let request = Request::Enrollment {
        request: EnrollmentRequest::Prepare {
            expected,
            document: ExportedInvitation::from_owned(Zeroizing::new(
                "qol-link:synthetic-canary".into(),
            ))
            .unwrap(),
        },
    };
    let payload = encode(request).unwrap();
    let written = Arc::new(Mutex::new(Vec::new()));
    let connection = ScriptedConnection {
        written: written.clone(),
        response: io::Cursor::new(Vec::new()),
        read_error: None,
    };
    assert_eq!(
        exchange(Box::new(connection), &payload, true, ADMIN_TIMEOUT),
        Err(PeerAdminClientError::OutcomeUnknown)
    );
    assert_eq!(*written.lock().unwrap(), *payload);
}
