use std::io::Write;
use std::time::Duration;

use qol_peers::admin::{Error, Request, Response};
use qol_runtime::local_ipc::LocalStream;

use crate::runtime::server::state_store::SharedState;

pub(super) fn handle(writer: &mut LocalStream, shared: &SharedState, request: Request) {
    if writer
        .set_write_timeout(Some(Duration::from_secs(10)))
        .is_err()
    {
        qol_runtime::probe!("TRAY_PEERS", "event=reply_transport outcome=unavailable");
        return;
    }
    let response = shared.peer_admin(request);
    let response = bounded_reply(response);
    let Ok(payload) = qol_runtime::local_ipc::encode_secret_json(&response) else {
        return;
    };
    if writer.write_all(&payload).is_err() {
        qol_runtime::probe!("TRAY_PEERS", "event=reply_delivery outcome=unknown");
    }
}

fn bounded_reply(response: Response) -> Response {
    match qol_runtime::local_ipc::encode_secret_json(&response) {
        Ok(_) => response,
        Err(_) => Response::Error {
            error: Error::ReplyTooLarge,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use std::sync::Arc;

    use super::*;
    use crate::features::linked_devices::tests::{attach, authority, host_at, status};
    use qol_peers::admin::{Lifecycle, PageCursor};
    use qol_runtime::protocol::RuntimeRequest;
    use qol_runtime::PlatformStateClient;

    fn route(shared: &SharedState, request: Request) -> Response {
        let (server, mut client) = LocalStream::pair().unwrap();
        let mut payload = serde_json::to_vec(&RuntimeRequest::PeerAdmin { request }).unwrap();
        payload.push(b'\n');
        client.write_all(&payload).unwrap();
        super::super::super::handle_connection(server, shared);
        let line = qol_runtime::local_ipc::read_line(&mut BufReader::new(client))
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn typed_socket_route_observes_the_same_host_and_preserves_old_encoding() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        let host = host_at(&root, false);
        let shared = attach(&host);
        assert_eq!(
            serde_json::to_string(&RuntimeRequest::GetState).unwrap(),
            r#"{"cmd":"get_state"}"#
        );
        assert!(matches!(
            route(
                &shared,
                Request::StartSession {
                    name: "local".into()
                }
            ),
            Response::Status { .. }
        ));
        let first = authority(&shared);
        let cursor = PageCursor {
            authority_id: first.peer_id,
            activation_id: first.activation_id,
            revision: first.revision,
            offset: 0,
        };
        let Response::Peers { page } = route(&shared, Request::Peers { cursor }) else {
            panic!("peer page");
        };
        assert_eq!(page.total, 0);
        assert_eq!(page.next, None);
        assert!(matches!(
            route(
                &shared,
                Request::Rename {
                    expected: first.expected(),
                    name: "changed".into()
                }
            ),
            Response::Changed { .. }
        ));
        assert_eq!(authority(&shared).name, "changed");
        assert_eq!(
            route(&shared, Request::Peers { cursor }),
            Response::Error {
                error: Error::StaleCursor
            }
        );
        route(
            &shared,
            Request::Stop {
                expected: authority(&shared).expected(),
            },
        );
        assert_eq!(status(&shared).lifecycle, Lifecycle::Inactive);
        host.shutdown();
        assert_eq!(
            route(
                &shared,
                Request::StartSession {
                    name: "late".into()
                }
            ),
            Response::Error {
                error: Error::Shutdown
            }
        );
        assert!(!root.exists());
    }

    #[test]
    fn real_runtime_client_attaches_through_the_credential_checked_dispatcher() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("runtime.sock");
        let listener = qol_runtime::local_ipc::LocalListener::bind(&path).unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = Arc::new(attach(&host));
        let dispatch_state = shared.clone();
        let worker = std::thread::spawn(move || {
            let dispatcher =
                super::super::super::ConnectionDispatcher::with_worker_count(dispatch_state, 1);
            dispatcher.dispatch(listener.accept().map(|(stream, _)| stream));
        });
        let client = PlatformStateClient::new(path);
        let reply = client
            .peer_admin(Request::StartSession {
                name: "local".into(),
            })
            .unwrap();
        worker.join().unwrap();
        let Response::Status { status: reply } = reply else {
            panic!("status");
        };
        assert_eq!(reply.authority.unwrap(), authority(&shared));
        host.shutdown();
    }

    #[test]
    fn missing_host_and_oversized_responses_are_explicit() {
        assert_eq!(
            route(&SharedState::new(Vec::new()), Request::Status),
            Response::Error {
                error: Error::HostUnavailable
            }
        );
        let temporary = tempfile::tempdir().unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = attach(&host);
        route(
            &shared,
            Request::StartSession {
                name: "local".into(),
            },
        );
        let mut status = status(&shared);
        status.authority.as_mut().unwrap().name =
            "x".repeat(qol_runtime::local_ipc::MAX_MESSAGE_BYTES);
        assert_eq!(
            bounded_reply(Response::Status { status }),
            Response::Error {
                error: Error::ReplyTooLarge
            }
        );
    }

    #[test]
    fn a_lost_mutation_reply_reports_unknown_after_the_real_route_commits_once() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("runtime.sock");
        let listener = qol_runtime::local_ipc::LocalListener::bind(&path).unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = Arc::new(attach(&host));
        route(
            &shared,
            Request::StartSession {
                name: "local".into(),
            },
        );
        let initial = authority(&shared);
        let server_state = shared.clone();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            qol_runtime::local_ipc::authorize_peer(&stream).unwrap();
            let request = qol_runtime::local_ipc::read_line(&mut BufReader::new(&stream))
                .unwrap()
                .unwrap();
            let (mut sink, _ignored_reply) = LocalStream::pair().unwrap();
            super::super::handle_request(&request, &mut sink, &server_state);
        });
        let client = PlatformStateClient::new(path);
        assert_eq!(
            client.peer_admin(Request::Rename {
                expected: initial.expected(),
                name: "committed".into()
            }),
            Err(qol_runtime::protocol::PeerAdminClientError::OutcomeUnknown)
        );
        worker.join().unwrap();
        let changed = authority(&shared);
        assert_eq!(changed.name, "committed");
        assert_eq!(changed.revision.value(), initial.revision.value() + 1);
        host.shutdown();
    }

    #[test]
    fn peer_admin_waits_past_the_platform_state_clients_short_timeout() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("runtime.sock");
        let listener = qol_runtime::local_ipc::LocalListener::bind(&path).unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = attach(&host);
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            qol_runtime::local_ipc::authorize_peer(&stream).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            super::super::super::handle_connection(stream, &shared);
        });
        let reply = PlatformStateClient::new(path)
            .peer_admin(Request::Status)
            .unwrap();
        worker.join().unwrap();
        assert!(
            matches!(reply, Response::Status { status } if status.lifecycle == Lifecycle::Inactive)
        );
        host.shutdown();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn socket_route_rejects_old_activation_stamps_after_persistent_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = attach(&host);
        route(
            &shared,
            Request::CreatePersistent {
                name: "local".into(),
            },
        );
        let old = authority(&shared);
        let expected = old.expected();
        route(&shared, Request::Stop { expected });
        route(&shared, Request::OpenPersistent);
        let current = authority(&shared);
        assert_eq!(old.peer_id, current.peer_id);
        assert_eq!(old.revision, current.revision);
        assert_ne!(old.activation_id, current.activation_id);
        for request in [
            Request::Rename {
                expected,
                name: "delayed".into(),
            },
            Request::SetGrants {
                expected,
                peer_id: old.peer_id,
                grants: vec![],
            },
            Request::Revoke {
                expected,
                peer_id: old.peer_id,
            },
            Request::Stop { expected },
        ] {
            assert_eq!(
                route(&shared, request),
                Response::Error {
                    error: Error::StaleAuthority
                }
            );
            assert_eq!(authority(&shared), current);
        }
        let cursor = PageCursor {
            authority_id: old.peer_id,
            activation_id: old.activation_id,
            revision: old.revision,
            offset: 0,
        };
        assert_eq!(
            route(&shared, Request::Peers { cursor }),
            Response::Error {
                error: Error::StaleCursor
            }
        );
        assert_eq!(
            route(
                &shared,
                Request::Rename {
                    expected: current.expected(),
                    name: "fresh".into(),
                }
            ),
            Response::Changed {
                authority_id: current.peer_id,
                activation_id: current.activation_id,
                revision: qol_peers::StoreRevision::new(1),
            }
        );
        host.shutdown();
    }
}

#[cfg(test)]
mod enrollment_tests {
    use crate::features::linked_devices::tests::enrollment::Fixture;
    use qol_peers::admin::{
        AttemptState, EnrollmentFailure, EnrollmentRequest, Error, PageCursor, Request, Response,
    };
    use qol_peers::enrollment::OutboundEnrollmentState;
    use std::net::Ipv4Addr;

    use super::peer_test_support::*;

    #[tokio::test]
    async fn two_attached_hosts_pair_through_real_local_clients_and_tls_with_empty_grants() {
        let temporary = tempfile::tempdir().unwrap();
        let inviter = Fixture::new(&temporary.path().join("inviter")).await;
        let joiner = Fixture::new(&temporary.path().join("joiner")).await;
        start(&inviter, false, "inviter").await;
        start(&joiner, false, "joiner").await;
        let Response::Invitation { document, .. } =
            call(&inviter, inviter.automatic_invitation_request()).await
        else {
            panic!("automatic invitation");
        };
        let imported =
            qol_peers::service::enrollment::Invitation::import(document.expose()).unwrap();
        assert_eq!(
            imported.endpoints(),
            &[(Ipv4Addr::LOCALHOST, inviter.ready().await).into()]
        );
        let Response::JoinPrepared { transaction, .. } = action(
            &joiner,
            EnrollmentRequest::Prepare {
                expected: joiner.expected(),
                document: document.clone(),
            },
        )
        .await
        else {
            panic!("prepare automatic invitation")
        };
        redeem(&joiner, document.clone(), transaction).await;
        let key = pending(&inviter).await;
        assert_eq!(key.peer, joiner.expected().authority_id);
        let Response::PendingEnrollments { items, .. } =
            action(&inviter, EnrollmentRequest::Pending {}).await
        else {
            panic!("pending");
        };
        assert_eq!(
            items[0].local_lifetime,
            qol_peers::AuthorityLifetime::Session
        );
        assert_eq!(
            items[0].remote_lifetime,
            qol_peers::AuthorityLifetime::Session
        );
        assert_eq!(items[0].name, "joiner");
        assert_eq!(
            action(
                &joiner,
                EnrollmentRequest::Redeem {
                    expected: joiner.expected(),
                    transaction,
                    document
                }
            )
            .await,
            Response::Error {
                error: Error::Enrollment {
                    error: EnrollmentFailure::AlreadyRunning
                }
            }
        );
        assert!(matches!(
            action(
                &inviter,
                EnrollmentRequest::Approve {
                    expected: inviter.expected(),
                    key
                }
            )
            .await,
            Response::Changed { .. }
        ));
        assert!(matches!(
            finished(&joiner, transaction).await,
            AttemptState::Completed { .. }
        ));
        for fixture in [&inviter, &joiner] {
            let expected = fixture.expected();
            let cursor = PageCursor {
                authority_id: expected.authority_id,
                activation_id: expected.activation_id,
                revision: expected.revision,
                offset: 0,
            };
            let Response::Peers { page } = call(fixture, Request::Peers { cursor }).await else {
                panic!("peers");
            };
            assert_eq!(page.total, 1);
            assert_eq!(page.items[0].grant_count, 0);
        }
        let Response::OutboundEnrollments { page } = action(
            &joiner,
            EnrollmentRequest::Outbound {
                cursor: PageCursor {
                    authority_id: joiner.expected().authority_id,
                    activation_id: joiner.expected().activation_id,
                    revision: joiner.expected().revision,
                    offset: 0,
                },
            },
        )
        .await
        else {
            panic!("outbound");
        };
        assert!(matches!(
            page.items[0].state,
            OutboundEnrollmentState::Committed { .. }
        ));
        assert!(matches!(
            call(
                &inviter,
                Request::Revoke {
                    expected: inviter.expected(),
                    peer_id: joiner.expected().authority_id,
                }
            )
            .await,
            Response::Changed { .. }
        ));
        let port = inviter.ready().await;
        action(
            &joiner,
            EnrollmentRequest::Recover {
                expected: joiner.expected(),
                transaction,
                endpoints: vec![(Ipv4Addr::LOCALHOST, port).into()],
            },
        )
        .await;
        assert!(!matches!(
            finished(&joiner, transaction).await,
            AttemptState::Completed { .. }
        ));
        let Response::Status { status } = call(&inviter, Request::Status).await else {
            panic!("status");
        };
        assert_eq!(status.authority.unwrap().peer_count, 0);
        inviter.close().await;
        joiner.close().await;
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn rejection_cancellation_and_abandonment_cannot_create_implicit_links() {
        for mode in ["reject", "cancel", "abandon"] {
            let temporary = tempfile::tempdir().unwrap();
            let inviter = Fixture::new(&temporary.path().join("inviter")).await;
            let joiner = Fixture::new(&temporary.path().join("joiner")).await;
            start(&inviter, false, "inviter").await;
            start(&joiner, false, "joiner").await;
            let (document, transaction) = prepare(&inviter, &joiner).await;
            redeem(&joiner, document, transaction).await;
            let key = pending(&inviter).await;
            let response = match mode {
                "reject" => {
                    action(
                        &inviter,
                        EnrollmentRequest::Reject {
                            expected: inviter.expected(),
                            key,
                        },
                    )
                    .await
                }
                "cancel" => {
                    action(
                        &inviter,
                        EnrollmentRequest::CancelInvitation {
                            expected: inviter.expected(),
                            invitation: key.invitation,
                        },
                    )
                    .await
                }
                "abandon" => {
                    action(
                        &joiner,
                        EnrollmentRequest::Abandon {
                            expected: joiner.expected(),
                            transaction,
                        },
                    )
                    .await
                }
                _ => unreachable!(),
            };
            assert!(matches!(response, Response::Changed { .. }), "{mode}");
            assert!(
                !matches!(
                    finished(&joiner, transaction).await,
                    AttemptState::Completed { .. }
                ),
                "{mode}"
            );
            for fixture in [&inviter, &joiner] {
                let Response::Status { status } = call(fixture, Request::Status).await else {
                    panic!("status");
                };
                assert_eq!(status.authority.unwrap().peer_count, 0, "{mode}");
            }
            if mode == "abandon" {
                assert!(matches!(
                    action(
                        &joiner,
                        EnrollmentRequest::Resume {
                            expected: joiner.expected(),
                            transaction
                        }
                    )
                    .await,
                    Response::Changed { .. }
                ));
                assert!(matches!(
                    action(
                        &inviter,
                        EnrollmentRequest::Approve {
                            expected: inviter.expected(),
                            key
                        }
                    )
                    .await,
                    Response::Changed { .. }
                ));
                let port = inviter.ready().await;
                action(
                    &joiner,
                    EnrollmentRequest::Recover {
                        expected: joiner.expected(),
                        transaction,
                        endpoints: vec![(Ipv4Addr::LOCALHOST, port).into()],
                    },
                )
                .await;
                assert!(matches!(
                    finished(&joiner, transaction).await,
                    AttemptState::Completed { .. }
                ));
            }
            inviter.close().await;
            joiner.close().await;
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn lost_admission_reply_restart_and_explicit_recovery_keep_original_pin_and_transaction()
    {
        let temporary = tempfile::tempdir().unwrap();
        let inviter_root = temporary.path().join("inviter");
        let joiner_root = temporary.path().join("joiner");
        let inviter = Fixture::new(&inviter_root).await;
        let joiner = Fixture::new(&joiner_root).await;
        start(&inviter, true, "inviter").await;
        start(&joiner, true, "joiner").await;
        let (document, transaction) = prepare(&inviter, &joiner).await;
        let old = joiner.expected();
        assert_eq!(
            local_call(
                joiner.shared.clone(),
                Request::Enrollment {
                    request: EnrollmentRequest::Redeem {
                        expected: old,
                        transaction,
                        document,
                    }
                },
                true
            )
            .await,
            Err(qol_runtime::protocol::PeerAdminClientError::OutcomeUnknown)
        );
        let key = pending(&inviter).await;
        joiner.close().await;
        action(
            &inviter,
            EnrollmentRequest::Approve {
                expected: inviter.expected(),
                key,
            },
        )
        .await;
        inviter.close().await;
        let inviter = Fixture::new(&inviter_root).await;
        let joiner = Fixture::new(&joiner_root).await;
        let port = inviter.ready().await;
        joiner.ready().await;
        assert_eq!(joiner.expected().authority_id, old.authority_id);
        assert_ne!(joiner.expected().activation_id, old.activation_id);
        assert_eq!(
            action(
                &joiner,
                EnrollmentRequest::Attempt {
                    expected: old,
                    transaction
                }
            )
            .await,
            Response::Error {
                error: Error::StaleAuthority
            }
        );
        assert!(matches!(
            action(
                &joiner,
                EnrollmentRequest::Attempt {
                    expected: joiner.expected(),
                    transaction
                }
            )
            .await,
            Response::EnrollmentAttempt {
                state: AttemptState::Unavailable {},
                ..
            }
        ));
        let stranger = Fixture::new(&temporary.path().join("stranger")).await;
        start(&stranger, false, "stranger").await;
        let wrong_port = stranger.ready().await;
        action(
            &joiner,
            EnrollmentRequest::Recover {
                expected: joiner.expected(),
                transaction,
                endpoints: vec![(Ipv4Addr::LOCALHOST, wrong_port).into()],
            },
        )
        .await;
        assert!(matches!(
            finished(&joiner, transaction).await,
            AttemptState::Unknown { .. }
        ));
        let Response::PendingEnrollments { items, .. } =
            action(&stranger, EnrollmentRequest::Pending {}).await
        else {
            panic!("pending");
        };
        assert!(items.is_empty());
        action(
            &joiner,
            EnrollmentRequest::Recover {
                expected: joiner.expected(),
                transaction,
                endpoints: vec![(Ipv4Addr::LOCALHOST, port).into()],
            },
        )
        .await;
        assert!(matches!(
            finished(&joiner, transaction).await,
            AttemptState::Completed { .. }
        ));
        stranger.close().await;
        inviter.close().await;
        joiner.close().await;
        for root in [inviter_root, joiner_root] {
            let writer = qol_peers::service::PeerAuthority::open_persistent(
                &root,
                std::time::SystemTime::now(),
            )
            .unwrap();
            assert_eq!(writer.projection().unwrap().peers.len(), 1);
        }
    }
}

pub(super) fn handle_operation(
    writer: &mut LocalStream,
    shared: &SharedState,
    request: qol_peers::operations::Request,
) {
    use qol_peers::operations::{Failure, Response};
    if writer
        .set_write_timeout(Some(Duration::from_secs(12)))
        .is_err()
    {
        return;
    }
    let response = match shared.peers() {
        Some(host) => host.operation_request(request),
        None => Response::Error {
            error: Failure::Unavailable,
        },
    };
    let payload = qol_runtime::local_ipc::encode_secret_json(&response).or_else(|_| {
        qol_runtime::local_ipc::encode_secret_json(&Response::Unknown { handle: None })
    });
    if let Ok(payload) = payload {
        if writer.write_all(&payload).is_err() {
            qol_runtime::probe!("TRAY_PEERS", "event=operation_local_reply outcome=unknown");
        }
    }
}

#[cfg(test)]
#[path = "peer_test_support.rs"]
mod peer_test_support;

#[cfg(all(test, target_os = "linux"))]
#[path = "peer_operation_tests.rs"]
mod operation_tests;
