use qol_peers::admin::{ActivationId, ExpectedAuthority, PageCursor};
use qol_peers::PeerId;

use super::*;

fn mutations(expected: ExpectedAuthority, peer_id: PeerId) -> [Request; 4] {
    [
        Request::Rename {
            expected,
            name: "delayed".into(),
        },
        Request::SetGrants {
            expected,
            peer_id,
            grants: vec![],
        },
        Request::Revoke { expected, peer_id },
        Request::Stop { expected },
    ]
}

fn reject_old_activation(shared: &SharedState, old: &AuthoritySummary, peer_id: PeerId) {
    let current = authority(shared);
    for expected in [
        old.expected(),
        ExpectedAuthority {
            authority_id: old.peer_id,
            ..current.expected()
        },
        ExpectedAuthority {
            activation_id: old.activation_id,
            ..current.expected()
        },
    ] {
        if expected == current.expected() {
            continue;
        }
        for request in mutations(expected, peer_id) {
            assert_eq!(
                shared.peer_admin(request),
                Response::Error {
                    error: Error::StaleAuthority
                }
            );
            assert_eq!(authority(shared), current);
        }
    }
    let cursor = PageCursor {
        authority_id: old.peer_id,
        activation_id: old.activation_id,
        revision: old.revision,
        offset: 0,
    };
    for request in [
        Request::Peers { cursor },
        Request::Grants { peer_id, cursor },
        Request::Tombstones { cursor },
    ] {
        assert_eq!(
            shared.peer_admin(request),
            Response::Error {
                error: Error::StaleCursor
            }
        );
    }
}

#[test]
fn delayed_mutations_cannot_cross_session_replacement_at_the_same_revision() {
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = attach(&host);
    shared.peer_admin(Request::StartSession {
        name: "first".into(),
    });
    let old = authority(&shared);
    let queued = attach(&host.clone());
    let (release, wait) = std::sync::mpsc::channel();
    let delayed = old.clone();
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        reject_old_activation(&queued, &delayed, delayed.peer_id);
    });
    shared.peer_admin(Request::Stop {
        expected: old.expected(),
    });
    shared.peer_admin(Request::StartSession {
        name: "second".into(),
    });
    let current = authority(&shared);
    assert_eq!(old.revision, current.revision);
    assert_ne!(old.peer_id, current.peer_id);
    assert_ne!(old.activation_id, current.activation_id);
    assert_eq!(authority(&attach(&host.clone())), current);
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(
        shared.peer_admin(Request::Rename {
            expected: current.expected(),
            name: "fresh".into()
        }),
        Response::Changed {
            authority_id: current.peer_id,
            activation_id: current.activation_id,
            revision: StoreRevision::new(1),
        }
    );
    assert!(matches!(
        shared.peer_admin(Request::Stop {
            expected: authority(&shared).expected()
        }),
        Response::Status {
            status: Status {
                lifecycle: Lifecycle::Inactive,
                ..
            }
        }
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn persistent_reopen_invalidates_stamps_and_pages_without_changing_identity_or_revision() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer_id = super::persistent::populated(&root, 1, 0)[0];
    let host = host_at(&root, false);
    let shared = attach(&host);
    let old = authority(&shared);
    let snapshot = std::fs::read(root.join("state.json")).unwrap();
    shared.peer_admin(Request::Stop {
        expected: old.expected(),
    });
    shared.peer_admin(Request::OpenPersistent);
    let current = authority(&shared);
    assert_eq!(current.peer_id, old.peer_id);
    assert_eq!(current.revision, old.revision);
    assert_ne!(current.activation_id, old.activation_id);
    reject_old_activation(&shared, &old, peer_id);
    assert_eq!(std::fs::read(root.join("state.json")).unwrap(), snapshot);
    let cursor = PageCursor {
        authority_id: current.peer_id,
        activation_id: current.activation_id,
        revision: current.revision,
        offset: 0,
    };
    for request in [
        Request::Peers { cursor },
        Request::Grants { peer_id, cursor },
        Request::Tombstones { cursor },
    ] {
        assert!(!matches!(
            shared.peer_admin(request),
            Response::Error { .. }
        ));
    }
    for request in [
        Request::SetGrants {
            expected: current.expected(),
            peer_id,
            grants: vec![],
        },
        Request::Revoke {
            expected: ExpectedAuthority {
                revision: StoreRevision::new(current.revision.value() + 1),
                ..current.expected()
            },
            peer_id,
        },
    ] {
        let Response::Changed { activation_id, .. } = shared.peer_admin(request) else {
            panic!("fresh mutation rejected");
        };
        assert_eq!(activation_id, current.activation_id);
    }
    assert_eq!(authority(&shared).peer_count, 0);
    assert_eq!(authority(&shared).tombstone_count, 1);
}

#[test]
fn randomness_failure_precedes_every_explicit_constructor_and_creates_no_storage() {
    for request in [
        Request::StartSession {
            name: "session".into(),
        },
        Request::CreatePersistent {
            name: "persistent".into(),
        },
        Request::OpenPersistent,
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        let host = host_at(&root, false);
        host.inner.lock().unwrap().activation = || Err(Error::ActivationUnavailable);
        let shared = attach(&host);
        assert_eq!(
            shared.peer_admin(request),
            Response::Error {
                error: Error::ActivationUnavailable
            }
        );
        assert_eq!(
            status(&shared).lifecycle,
            Lifecycle::Unavailable {
                error: Error::ActivationUnavailable
            }
        );
        assert!(!root.exists());
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
        host.inner.lock().unwrap().activation = super::super::activation_id;
        shared.peer_admin(Request::StartSession {
            name: "recovered".into(),
        });
        assert_eq!(status(&shared).lifecycle, Lifecycle::Active);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn randomness_failure_during_startup_does_not_acquire_the_existing_writer() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    super::persistent::populated(&root, 0, 0);
    let snapshot = std::fs::read(root.join("state.json")).unwrap();
    let host = PeerHostHandle::new(
        Ok(root.clone()),
        Arc::new(Mutex::new(PluginManager::new())),
        false,
    );
    host.inner.lock().unwrap().activation = || Err(Error::ActivationUnavailable);
    host.initialize();
    assert_eq!(
        status(&attach(&host)).lifecycle,
        Lifecycle::Unavailable {
            error: Error::ActivationUnavailable
        }
    );
    let opened =
        qol_peers::service::PeerAuthority::open_persistent(&root, std::time::SystemTime::now())
            .unwrap();
    assert_eq!(std::fs::read(root.join("state.json")).unwrap(), snapshot);
    drop(opened);
}

#[test]
fn page_activation_mismatch_is_rejected_independently_of_identity_and_revision() {
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = attach(&host);
    shared.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    let current = authority(&shared);
    let activation_id = if current.activation_id == ActivationId::from_bytes([0; 16]) {
        ActivationId::from_bytes([1; 16])
    } else {
        ActivationId::from_bytes([0; 16])
    };
    assert_eq!(
        shared.peer_admin(Request::Peers {
            cursor: PageCursor {
                authority_id: current.peer_id,
                activation_id,
                revision: current.revision,
                offset: 0,
            }
        }),
        Response::Error {
            error: Error::StaleCursor
        }
    );
}
