use crate::features::linked_devices::tests::enrollment::Fixture;
use crate::runtime::SharedState;
use qol_peers::admin::{AttemptState, EnrollmentRequest, Request, Response};
use qol_peers::enrollment::{EnrollmentRequestKey, ExportedInvitation, TransactionId};
use qol_runtime::{protocol::PeerAdminClientError, PlatformStateClient};
use std::{io::BufReader, net::Ipv4Addr, sync::Arc, time::Duration};
pub(super) async fn call(fixture: &Fixture, request: Request) -> Response {
    local_call(fixture.shared.clone(), request, false)
        .await
        .unwrap()
}

pub(super) async fn local_call(
    shared: Arc<SharedState>,
    request: Request,
    lose_reply: bool,
) -> Result<Response, PeerAdminClientError> {
    tokio::task::spawn_blocking(move || {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("runtime.sock");
        let listener = qol_runtime::local_ipc::bind_listener(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            if lose_reply {
                qol_runtime::local_ipc::authorize_peer(&stream).unwrap();
                let request =
                    qol_runtime::local_ipc::read_secret_line(&mut BufReader::new(&stream))
                        .unwrap()
                        .unwrap();
                let (mut sink, _reply) = qol_runtime::local_ipc::LocalStream::pair().unwrap();
                super::super::handle_request(&request, &mut sink, &shared);
                return;
            }
            let dispatcher =
                super::super::super::ConnectionDispatcher::with_worker_count(shared, 1);
            dispatcher.dispatch(Ok(stream));
        });
        let result = PlatformStateClient::new(path).peer_admin(request);
        worker.join().unwrap();
        result
    })
    .await
    .unwrap()
}

pub(super) async fn action(fixture: &Fixture, request: EnrollmentRequest) -> Response {
    call(fixture, Request::Enrollment { request }).await
}

pub(super) async fn start(fixture: &Fixture, persistent: bool, name: &str) {
    let request = if persistent {
        Request::CreatePersistent { name: name.into() }
    } else {
        Request::StartSession { name: name.into() }
    };
    assert!(matches!(
        call(fixture, request).await,
        Response::Status { .. }
    ));
    fixture.ready().await;
}

pub(super) async fn prepare(
    inviter: &Fixture,
    joiner: &Fixture,
) -> (ExportedInvitation, TransactionId) {
    let Response::Invitation { document, .. } = action(
        inviter,
        EnrollmentRequest::CreateInvitation {
            expected: inviter.expected(),
            addresses: vec![Ipv4Addr::LOCALHOST.into()],
        },
    )
    .await
    else {
        panic!("invitation");
    };
    let Response::JoinPrepared { transaction, .. } = action(
        joiner,
        EnrollmentRequest::Prepare {
            expected: joiner.expected(),
            document: document.clone(),
        },
    )
    .await
    else {
        panic!("prepared");
    };
    (document, transaction)
}

pub(super) async fn redeem(
    joiner: &Fixture,
    document: ExportedInvitation,
    transaction: TransactionId,
) {
    let response = action(
        joiner,
        EnrollmentRequest::Redeem {
            expected: joiner.expected(),
            transaction,
            document,
        },
    )
    .await;
    assert!(
        matches!(response, Response::EnrollmentAttempt { state: AttemptState::Queued {}, transaction: actual, .. } if actual == transaction)
    );
}

pub(super) async fn pending(inviter: &Fixture) -> EnrollmentRequestKey {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Response::PendingEnrollments { items, .. } =
                action(inviter, EnrollmentRequest::Pending {}).await
            else {
                panic!("pending");
            };
            if let Some(item) = items.first() {
                assert!(!item.name.is_empty());
                return item.key;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

pub(super) async fn finished(joiner: &Fixture, transaction: TransactionId) -> AttemptState {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Response::EnrollmentAttempt { state, .. } = action(
                joiner,
                EnrollmentRequest::Attempt {
                    expected: joiner.expected(),
                    transaction,
                },
            )
            .await
            else {
                panic!("attempt");
            };
            if !state.is_active() {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
