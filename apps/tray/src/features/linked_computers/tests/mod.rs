mod activation;
pub(crate) mod enrollment;
mod network;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod network_pages;
mod pagination;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod persistent;

use std::path::Path;
use std::sync::{Arc, Mutex};

use qol_peers::admin::{AuthoritySummary, Error, Lifecycle, Request, Response, Status};
use qol_peers::{AuthorityError, AuthorityLifetime, StoreRevision};

use super::{LinkedComputers, PeerHostHandle};
use crate::plugins::PluginManager;
use crate::runtime::SharedState;

pub(crate) fn host_at(root: &Path, shadow: bool) -> PeerHostHandle {
    let host = PeerHostHandle::new(
        Ok(root.to_path_buf()),
        Arc::new(Mutex::new(PluginManager::new())),
        shadow,
    );
    host.initialize();
    host
}

pub(crate) fn attach(host: &PeerHostHandle) -> SharedState {
    let shared = SharedState::new(Vec::new());
    assert!(shared.attach_peers(host.clone()));
    shared
}

pub(crate) fn status(shared: &SharedState) -> Status {
    let Response::Status { status } = shared.peer_admin(Request::Status) else {
        panic!("status response required");
    };
    status
}

pub(crate) fn authority(shared: &SharedState) -> AuthoritySummary {
    status(shared).authority.expect("active authority")
}

#[test]
fn session_handles_share_revisions_and_stop_without_creating_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let host = host_at(&root, false);
    let first = attach(&host);
    let second = attach(&host.clone());
    assert_eq!(status(&first).lifecycle, Lifecycle::Inactive);
    first.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    let initial = authority(&second);
    assert_eq!(initial.lifetime, AuthorityLifetime::Session);
    let expected = initial.expected();
    assert!(matches!(
        second.peer_admin(Request::Rename {
            expected,
            name: "renamed".into()
        }),
        Response::Changed { .. }
    ));
    assert_eq!(authority(&first).name, "renamed");
    assert_eq!(
        first.peer_admin(Request::Stop { expected }),
        Response::Error {
            error: AuthorityError::StaleRevision {
                expected: expected.revision,
                current: StoreRevision::new(1)
            }
            .into(),
        }
    );
    first.peer_admin(Request::Stop {
        expected: authority(&first).expected(),
    });
    assert_eq!(
        status(&second),
        Status {
            lifecycle: Lifecycle::Inactive,
            authority: None
        }
    );
    second.peer_admin(Request::StartSession {
        name: "next".into(),
    });
    assert_ne!(authority(&first).peer_id, initial.peer_id);
    host.shutdown();
    host.shutdown();
    assert_eq!(status(&first).lifecycle, Lifecycle::Shutdown);
    assert_eq!(
        first.peer_admin(Request::StartSession {
            name: "late".into()
        }),
        Response::Error {
            error: Error::Shutdown
        }
    );
    assert!(!root.exists());
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn owned_shutdown_observer_clears_the_handle_and_pending_clones() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let (tx, rx) = tokio::sync::broadcast::channel(1);
    let mut owner = LinkedComputers::start_at(
        Ok(root.clone()),
        Arc::new(Mutex::new(PluginManager::new())),
        false,
        rx,
        None,
    )
    .await;
    let pending = owner.handle();
    let shared = attach(&pending);
    shared.peer_admin(Request::StartSession {
        name: "session".into(),
    });
    tx.send(()).unwrap();
    owner.shutdown_task.take().unwrap().await.unwrap().unwrap();
    assert_eq!(status(&shared).lifecycle, Lifecycle::Shutdown);
    assert_eq!(
        pending.request(Request::OpenPersistent),
        Response::Error {
            error: Error::Shutdown
        }
    );
    assert!(!root.exists());
}

#[tokio::test]
async fn dropping_the_owner_closes_an_attached_session() {
    let temporary = tempfile::tempdir().unwrap();
    let (_tx, rx) = tokio::sync::broadcast::channel(1);
    let owner = LinkedComputers::start_at(
        Ok(temporary.path().join("peers")),
        Arc::new(Mutex::new(PluginManager::new())),
        false,
        rx,
        None,
    )
    .await;
    let shared = attach(&owner.handle());
    shared.peer_admin(Request::StartSession {
        name: "session".into(),
    });
    drop(owner);
    assert_eq!(status(&shared).lifecycle, Lifecycle::Shutdown);
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn standby_refuses_all_authority_constructors_until_promotion() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let host = host_at(&root, true);
    let shared = attach(&host);
    for request in [
        Request::StartSession { name: "x".into() },
        Request::CreatePersistent { name: "x".into() },
        Request::OpenPersistent,
    ] {
        assert_eq!(
            shared.peer_admin(request),
            Response::Error {
                error: Error::Standby
            }
        );
    }
    assert_eq!(status(&shared).lifecycle, Lifecycle::Standby);
    assert!(!root.exists());
    host.promote();
    assert_eq!(status(&shared).lifecycle, Lifecycle::Inactive);
    shared.peer_admin(Request::StartSession { name: "x".into() });
    let initial = authority(&shared);
    host.promote();
    assert_eq!(authority(&shared), initial);
}

#[test]
fn root_resolution_failure_is_visible_and_explicit_session_still_works() {
    let host = PeerHostHandle::new(
        Err(Error::RootUnavailable),
        Arc::new(Mutex::new(PluginManager::new())),
        false,
    );
    host.initialize();
    let shared = attach(&host);
    assert_eq!(
        status(&shared).lifecycle,
        Lifecycle::Unavailable {
            error: Error::RootUnavailable
        }
    );
    shared.peer_admin(Request::StartSession {
        name: "session".into(),
    });
    assert_eq!(authority(&shared).lifetime, AuthorityLifetime::Session);
}

#[test]
fn a_queued_local_task_cannot_reopen_an_authority_after_shutdown() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let host = host_at(&root, false);
    let shared = attach(&host);
    shared.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    let queued = host.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        wait.recv().unwrap();
        queued.request(Request::StartSession {
            name: "late".into(),
        })
    });
    host.shutdown();
    release.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap(),
        Response::Error {
            error: Error::Shutdown
        }
    );
    assert_eq!(status(&shared).authority, None);
    assert!(!root.exists());
}

#[cfg(not(any(any(target_os = "linux", target_os = "macos"), target_os = "macos")))]
#[test]
fn unsupported_persistence_is_explicit_and_does_not_create_a_store() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let host = host_at(&root, false);
    let shared = attach(&host);
    let error = Error::from(AuthorityError::UnsupportedPlatform);
    assert_eq!(
        shared.peer_admin(Request::CreatePersistent {
            name: "local".into()
        }),
        Response::Error { error }
    );
    assert_eq!(status(&shared).lifecycle, Lifecycle::Unavailable { error });
    assert!(!root.exists());
    shared.peer_admin(Request::StartSession {
        name: "local".into(),
    });
    assert_eq!(authority(&shared).lifetime, AuthorityLifetime::Session);
}
