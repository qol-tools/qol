use std::{
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use qol_peers::admin::{EnrollmentRequest, ExpectedAuthority, Request, Response};
use qol_peers::network::ListenerStatus;
use qol_peers::service::network::{
    discovery::{Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture},
    NetworkOptions,
};
use tokio::sync::{mpsc, watch};

use super::{attach, authority, LinkedDevices, SharedState};
use crate::plugins::PluginManager;

struct Discovery;

impl DiscoveryFactory for Discovery {
    fn run(
        &self,
        _: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        Box::pin(async move {
            let _ = events.send(DiscoveryEvent::Ready).await;
            while !*stop.borrow_and_update() {
                if stop.changed().await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    }
}

pub(crate) struct Fixture {
    pub owner: LinkedDevices,
    pub shared: Arc<SharedState>,
    _shutdown: tokio::sync::broadcast::Sender<()>,
}

impl Fixture {
    pub(crate) async fn new(root: &Path) -> Self {
        Self::with_discovery(root, Arc::new(Discovery)).await
    }

    pub(crate) async fn with_discovery(root: &Path, discovery: Arc<dyn DiscoveryFactory>) -> Self {
        let (shutdown, receiver) = tokio::sync::broadcast::channel(1);
        let owner = LinkedDevices::start_at(
            Ok(root.into()),
            Arc::new(Mutex::new(PluginManager::new())),
            false,
            receiver,
            Some(NetworkOptions {
                bind: Ipv4Addr::LOCALHOST,
                allow_loopback_hints: true,
                discovery,
            }),
        )
        .await;
        let shared = Arc::new(attach(&owner.handle()));
        Self {
            owner,
            shared,
            _shutdown: shutdown,
        }
    }

    pub(crate) fn automatic_invitation_request(&self) -> Request {
        let mut request = Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected: self.expected(),
                addresses: Vec::new(),
            },
        };
        super::super::enrollment::preflight_addresses(&mut request, || {
            Ok(vec![Ipv4Addr::LOCALHOST.into()])
        })
        .unwrap();
        request
    }

    pub(crate) fn expected(&self) -> ExpectedAuthority {
        authority(&self.shared).expected()
    }

    pub(crate) async fn ready(&self) -> u16 {
        let mut view = self
            .owner
            .handle
            .inner
            .lock()
            .unwrap()
            .authority()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ListenerStatus::Listening { port } = view.borrow().status.enrollment_listener
                {
                    return port;
                }
                view.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }

    pub(crate) async fn close(self) {
        self.owner.handle().shutdown_and_wait().await.unwrap();
        self.owner.closed().await.unwrap();
    }
}

#[tokio::test]
async fn enrollment_endpoints_and_stamps_fail_without_mutating_the_active_authority() {
    use qol_peers::admin::{ActivationId, EnrollmentFailure, Error};
    let temporary = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(&temporary.path().join("peers")).await;
    fixture.shared.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    fixture.ready().await;
    let expected = fixture.expected();
    for (addresses, error) in [
        (vec!["0.0.0.0"], EnrollmentFailure::InvalidEndpoint),
        (vec!["224.0.0.1"], EnrollmentFailure::InvalidEndpoint),
        (vec!["255.255.255.255"], EnrollmentFailure::InvalidEndpoint),
        (vec!["::1"], EnrollmentFailure::UnsupportedAddressFamily),
        (vec!["127.0.0.1"; 9], EnrollmentFailure::InvalidEndpoint),
    ] {
        let response = fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected,
                addresses: addresses
                    .iter()
                    .map(|address| address.parse().unwrap())
                    .collect(),
            },
        });
        assert_eq!(
            response,
            Response::Error {
                error: Error::Enrollment { error }
            },
            "{addresses:?}"
        );
        assert_eq!(fixture.expected(), expected, "{addresses:?}");
    }
    let stale = ExpectedAuthority {
        activation_id: ActivationId::from_bytes([9; 16]),
        ..expected
    };
    assert_eq!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected: stale,
                addresses: vec![Ipv4Addr::LOCALHOST.into()],
            }
        }),
        Response::Error {
            error: Error::StaleAuthority
        }
    );
    fixture.shared.peer_admin(Request::Rename {
        expected,
        name: "renamed".into(),
    });
    assert!(matches!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected,
                addresses: vec![Ipv4Addr::LOCALHOST.into()],
            }
        }),
        Response::Error {
            error: Error::Authority {
                error: qol_peers::AuthorityError::StaleRevision { .. }
            }
        }
    ));
    fixture.close().await;
}

#[tokio::test]
async fn invitation_capacity_and_unknown_transaction_are_typed_local_refusals() {
    use qol_peers::admin::{EnrollmentFailure, Error};
    use qol_peers::enrollment::EnrollmentRejection;
    let temporary = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(&temporary.path().join("peers")).await;
    fixture.shared.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    fixture.ready().await;
    let expected = fixture.expected();
    let mut first = None;
    for _ in 0..8 {
        let Response::Invitation { invitation, .. } =
            fixture.shared.peer_admin(Request::Enrollment {
                request: EnrollmentRequest::CreateInvitation {
                    expected,
                    addresses: vec![Ipv4Addr::LOCALHOST.into()],
                },
            })
        else {
            panic!("invitation");
        };
        first.get_or_insert(invitation);
    }
    assert_eq!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected,
                addresses: vec![Ipv4Addr::LOCALHOST.into()]
            },
        }),
        Response::Error {
            error: Error::Enrollment {
                error: EnrollmentFailure::Rejected(EnrollmentRejection::Capacity)
            }
        }
    );
    assert_eq!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::Attempt {
                expected,
                transaction: "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap(),
            }
        }),
        Response::Error {
            error: Error::Enrollment {
                error: EnrollmentFailure::UnknownTransaction
            }
        }
    );
    assert!(matches!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CancelInvitation {
                expected,
                invitation: first.unwrap(),
            }
        }),
        Response::Changed { .. }
    ));
    assert!(matches!(
        fixture.shared.peer_admin(Request::Enrollment {
            request: EnrollmentRequest::CreateInvitation {
                expected,
                addresses: vec![Ipv4Addr::LOCALHOST.into()]
            },
        }),
        Response::Invitation { .. }
    ));
    assert_eq!(fixture.expected(), expected);
    fixture.close().await;
}

impl Fixture {
    #[cfg(target_os = "linux")]
    pub(crate) fn plugins(&self) -> Arc<Mutex<PluginManager>> {
        self.owner.handle().plugins.clone()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn operations_ready(&self, peer: qol_peers::PeerId) -> bool {
        self.owner
            .handle()
            .inner
            .lock()
            .unwrap()
            .authority()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .operations_ready(peer)
    }
}

impl Fixture {
    #[cfg(target_os = "linux")]
    pub(crate) fn send_operation_fixture(
        &self,
        invocation: qol_peers::operations::Invocation,
    ) -> std::sync::mpsc::Receiver<
        Result<qol_peers::operations::Outcome, qol_peers::operations::Failure>,
    > {
        self.owner
            .handle()
            .inner
            .lock()
            .unwrap()
            .authority()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .invoke_operation(invocation)
            .unwrap()
    }
}

#[test]
fn address_preflight_is_injectable_refuses_failures_and_preserves_explicit_callers() {
    use qol_peers::admin::{ActivationId, EnrollmentFailure, Error};
    use qol_peers::service::Identity;
    let expected = ExpectedAuthority {
        authority_id: Identity::generate(std::time::SystemTime::now())
            .unwrap()
            .pin()
            .peer_id(),
        activation_id: ActivationId::from_bytes([8; 16]),
        revision: qol_peers::StoreRevision::INITIAL,
    };
    let mut request = Request::Enrollment {
        request: EnrollmentRequest::CreateInvitation {
            expected,
            addresses: Vec::new(),
        },
    };
    let unavailable = Error::Enrollment {
        error: EnrollmentFailure::Unavailable,
    };
    assert_eq!(
        super::super::enrollment::preflight_addresses(&mut request, || Err(unavailable)),
        Err(unavailable)
    );
    assert_eq!(
        super::super::enrollment::preflight_addresses(&mut request, || Ok(Vec::new())),
        Err(unavailable)
    );
    super::super::enrollment::preflight_addresses(&mut request, || {
        Ok(vec![Ipv4Addr::new(192, 168, 1, 7).into()])
    })
    .unwrap();
    super::super::enrollment::preflight_addresses(&mut request, || {
        panic!("explicit addresses must not enumerate interfaces")
    })
    .unwrap();
    let Request::Enrollment {
        request:
            EnrollmentRequest::CreateInvitation {
                expected: actual,
                addresses,
            },
    } = request
    else {
        panic!("invitation")
    };
    assert_eq!(actual, expected);
    assert_eq!(addresses, vec![std::net::IpAddr::from([192, 168, 1, 7])]);
}
