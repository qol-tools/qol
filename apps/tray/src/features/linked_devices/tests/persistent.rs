use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use qol_peers::admin::{PageCursor, Request, Response};
use qol_peers::service::{Identity, PeerAuthority};
use qol_peers::{AuthorityStatus, PeerId};
use qol_plugin_api::operations::{OperationKey, OperationKind};
use serde_json::json;

use super::*;

pub(super) fn populated(root: &Path, peers: usize) -> Vec<PeerId> {
    let authority =
        PeerAuthority::create_persistent(root, "local".into(), SystemTime::now()).unwrap();
    drop(authority);
    let path = root.join("state.json");
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let grants: Vec<_> = (0..128)
        .map(|index| {
            OperationKey::new(
                crate::plugins::manifest::PluginUid::new("u".repeat(256)),
                OperationKind::Query,
                format!("g{index:03}{}", "a".repeat(252)),
            )
        })
        .collect();
    let mut ids = Vec::new();
    snapshot["peers"] = json!((0..peers).map(|_| {
        let identity = Identity::generate(SystemTime::now()).unwrap();
        let pin = identity.pin();
        ids.push(pin.peer_id());
        json!({"pin": {"peer_id": pin.peer_id(), "spki": pin.spki_der()}, "name": "\\\"".repeat(128), "grants": grants})
    }).collect::<Vec<_>>());
    snapshot["version"] = json!(3);
    snapshot.as_object_mut().unwrap().remove("operations");
    fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    drop(PeerAuthority::open_persistent(root, SystemTime::now()).unwrap());
    ids
}

#[test]
fn editing_grants_preserves_selected_unavailable_permissions_but_cannot_add_unknown_ones() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer_id = populated(&root, 1)[0];
    let host = host_at(&root, false);
    let shared = attach(&host);
    let expected = authority(&shared).expected();
    let cursor = PageCursor {
        authority_id: expected.authority_id,
        activation_id: expected.activation_id,
        revision: expected.revision,
        offset: 0,
    };
    let Response::Grants { page, .. } = shared.peer_admin(Request::Grants { peer_id, cursor })
    else {
        panic!("grants");
    };
    let retained = page.items[..2].to_vec();
    assert!(matches!(
        shared.peer_admin(Request::SetGrants {
            expected,
            peer_id,
            grants: retained.clone(),
        }),
        Response::Changed { .. }
    ));
    let current = authority(&shared).expected();
    let mut added = retained.clone();
    added.push(OperationKey::new(
        crate::plugins::manifest::PluginUid::new("missing"),
        OperationKind::Action,
        "unknown",
    ));
    assert_eq!(
        shared.peer_admin(Request::SetGrants {
            expected: current,
            peer_id,
            grants: added,
        }),
        Response::Error {
            error: Error::GrantUnavailable
        }
    );
    assert_eq!(authority(&shared).expected(), current);
    let Response::Grants { page, .. } = shared.peer_admin(Request::Grants {
        peer_id,
        cursor: PageCursor {
            revision: current.revision,
            ..cursor
        },
    }) else {
        panic!("grants");
    };
    assert_eq!(page.items, retained);
    assert_eq!(page.total, 2);
    host.shutdown();
}

#[test]
fn populated_fixture_commits_migration_before_reopen_and_preserves_link_epochs() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let ids = populated(&root, 2);
    let path = root.join("state.json");
    let completed = fs::read(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&completed).unwrap();
    assert_eq!(stored["version"], 5);
    let authority = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    let original = authority.projection().unwrap();
    assert_eq!(original.revision, StoreRevision::new(1));
    assert_eq!(original.peers.len(), ids.len());
    for id in &ids {
        assert_eq!(
            original
                .peers
                .iter()
                .find(|peer| peer.peer_id == *id)
                .unwrap()
                .grants
                .len(),
            128
        );
    }
    let epochs: Vec<_> = ids
        .iter()
        .map(|id| authority.operation_epoch(*id).unwrap())
        .collect();
    assert_eq!(fs::read(&path).unwrap(), completed);
    drop(authority);
    let reopened = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    assert_eq!(reopened.projection().unwrap(), original);
    for (id, epoch) in ids.iter().zip(epochs) {
        assert_eq!(reopened.operation_epoch(*id).unwrap(), epoch);
    }
    assert_eq!(fs::read(path).unwrap(), completed);
}

#[test]
fn absent_incomplete_and_corrupt_roots_have_distinct_outcomes_without_repair() {
    for case in [
        "absent",
        "empty",
        "missing_snapshot",
        "missing_lock",
        "corrupt",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        match case {
            "absent" => {}
            "empty" => fs::DirBuilder::new().mode(0o700).create(&root).unwrap(),
            _ => {
                drop(
                    PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now())
                        .unwrap(),
                );
                match case {
                    "missing_snapshot" => fs::remove_file(root.join("state.json")).unwrap(),
                    "missing_lock" => fs::remove_file(root.join("writer.lock")).unwrap(),
                    "corrupt" => fs::write(root.join("state.json"), b"invalid").unwrap(),
                    _ => unreachable!(),
                }
            }
        }
        let before = fs::read(root.join("state.json")).ok();
        let host = host_at(&root, false);
        let view = status(&attach(&host));
        let expected = match case {
            "absent" => Lifecycle::Inactive,
            "corrupt" => Lifecycle::Unavailable {
                error: AuthorityError::InvalidSnapshot.into(),
            },
            _ => Lifecycle::Unavailable {
                error: AuthorityError::MissingStore.into(),
            },
        };
        assert_eq!(view.lifecycle, expected, "{case}");
        assert!(view.authority.is_none(), "{case}");
        assert_eq!(fs::read(root.join("state.json")).ok(), before, "{case}");
    }
}

#[test]
fn persistent_reopen_preserves_identity_grants_and_excludes_a_competing_writer() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peers = populated(&root, 1);
    let host = host_at(&root, false);
    let shared = attach(&host);
    let initial = authority(&shared);
    assert_eq!(initial.grant_count, 128);
    let competitor = host_at(&root, false);
    assert_eq!(
        status(&attach(&competitor)).lifecycle,
        Lifecycle::Unavailable {
            error: AuthorityError::WriterBusy.into()
        }
    );
    assert_eq!(
        shared.peer_admin(Request::StartSession {
            name: "other".into()
        }),
        Response::Error {
            error: Error::AlreadyActive
        }
    );
    shared.peer_admin(Request::Stop {
        expected: initial.expected(),
    });
    assert!(matches!(
        attach(&competitor).peer_admin(Request::OpenPersistent),
        Response::Status { .. }
    ));
    let reopened = attach(&competitor);
    assert_reopened(&authority(&reopened), &initial);
    assert!(matches!(
        reopened.peer_admin(Request::SetGrants {
            expected: authority(&reopened).expected(),
            peer_id: peers[0],
            grants: vec![]
        }),
        Response::Changed { .. }
    ));
    assert_eq!(authority(&reopened).grant_count, 0);
    competitor.shutdown();
    shared.peer_admin(Request::OpenPersistent);
    assert_eq!(authority(&shared).peer_id, initial.peer_id);
    assert_eq!(authority(&shared).grant_count, 0);
}

#[test]
fn promotion_opens_only_after_predecessor_shutdown_and_never_retries() {
    for release_first in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        populated(&root, 0);
        let predecessor = host_at(&root, false);
        let initial = authority(&attach(&predecessor));
        let shadow = host_at(&root, true);
        let shared = attach(&shadow);
        assert_eq!(status(&shared).lifecycle, Lifecycle::Standby);
        if release_first {
            predecessor.shutdown();
        }
        shadow.promote();
        if release_first {
            assert_reopened(&authority(&shared), &initial);
            continue;
        }
        let unavailable = Lifecycle::Unavailable {
            error: AuthorityError::WriterBusy.into(),
        };
        assert_eq!(status(&shared).lifecycle, unavailable);
        predecessor.shutdown();
        shadow.promote();
        assert_eq!(status(&shared).lifecycle, unavailable);
        shared.peer_admin(Request::OpenPersistent);
        assert_reopened(&authority(&shared), &initial);
    }
}

#[test]
fn faulted_authority_remains_visible_and_rejects_mutations_until_stop_and_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer = populated(&root, 1)[0];
    let host = host_at(&root, false);
    let shared = attach(&host);
    fs::set_permissions(root.join("state.json"), fs::Permissions::from_mode(0o644)).unwrap();
    let expected = authority(&shared).expected();
    assert!(matches!(
        shared.peer_admin(Request::Rename {
            expected,
            name: "fail".into()
        }),
        Response::Error { .. }
    ));
    assert_eq!(authority(&shared).status, AuthorityStatus::Faulted);
    assert_eq!(authority(&shared).revision, expected.revision);
    assert_eq!(
        shared.peer_admin(Request::Revoke {
            expected,
            peer_id: peer
        }),
        Response::Error {
            error: AuthorityError::Faulted.into()
        }
    );
    fs::set_permissions(root.join("state.json"), fs::Permissions::from_mode(0o600)).unwrap();
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
    shared.peer_admin(Request::OpenPersistent);
    assert_ne!(authority(&shared).activation_id, expected.activation_id);
    assert_eq!(authority(&shared).status, AuthorityStatus::Ready);
    assert_eq!(authority(&shared).name, "local");
}

#[test]
fn unlinking_and_stale_revisions_are_observed_through_the_shared_route() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer = populated(&root, 1)[0];
    let host = host_at(&root, false);
    let shared = attach(&host);
    let initial = authority(&shared);
    let expected = initial.expected();
    assert!(matches!(
        shared.peer_admin(Request::Revoke {
            expected,
            peer_id: peer
        }),
        Response::Changed { .. }
    ));
    assert_eq!(authority(&shared).peer_count, 0);
    assert_eq!(
        shared.peer_admin(Request::Rename {
            expected,
            name: "stale".into()
        }),
        Response::Error {
            error: AuthorityError::StaleRevision {
                expected: expected.revision,
                current: StoreRevision::new(expected.revision.value() + 1)
            }
            .into(),
        }
    );
    assert_eq!(
        shared.peer_admin(Request::SetGrants {
            expected: authority(&shared).expected(),
            peer_id: peer,
            grants: vec![]
        }),
        Response::Error {
            error: AuthorityError::UnknownPeer.into()
        }
    );
    assert_eq!(
        shared.peer_admin(Request::Grants {
            peer_id: peer,
            cursor: PageCursor {
                authority_id: initial.peer_id,
                activation_id: initial.activation_id,
                revision: StoreRevision::new(expected.revision.value() + 1),
                offset: 0
            }
        }),
        Response::Error {
            error: AuthorityError::UnknownPeer.into()
        }
    );
}

#[test]
fn profile_selection_does_not_change_authority_root_or_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(temporary.path());
    let base = crate::paths::base_data_dir().unwrap();
    fs::create_dir_all(&base).unwrap();
    let root = base.join("peers");
    populated(&root, 1);
    let host = host_at(&root, false);
    let shared = attach(&host);
    let initial = authority(&shared);
    let snapshot = fs::read(root.join("state.json")).unwrap();
    for profile in ["alpha", "beta"] {
        let marker = crate::paths::active_profile_marker_path().unwrap();
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(marker, profile).unwrap();
        assert_eq!(crate::paths::active_profile_name(), profile);
        assert_eq!(crate::paths::base_data_dir().unwrap().join("peers"), root);
        assert_eq!(authority(&shared), initial);
        assert_eq!(fs::read(root.join("state.json")).unwrap(), snapshot);
    }
}

#[test]
fn grant_replacement_uses_current_exact_exposure_and_allows_clear_after_plugin_removal() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer = populated(&root, 1)[0];
    let plugin_root = temporary.path().join("plugin");
    fs::create_dir(&plugin_root).unwrap();
    fs::write(plugin_root.join("plugin-bin"), "fixture").unwrap();
    let host = host_at(&root, false);
    let shared = attach(&host);
    let manifest: crate::plugins::manifest::PluginManifest = toml::from_str(
        r#"
manifest_version = 2
[plugin]
id = "qol-fixture"
uid = "fixture-uid"
name = "Fixture"
version = "1.0.0"
description = ""
[menu]
label = "Fixture"
items = []
[runtime]
command = "plugin-bin"
[action.run]
label = "Run"
kind = "run"
args = ["run"]
"#,
    )
    .unwrap();
    let plugin = crate::plugins::Plugin::new(
        crate::plugins::manifest::PluginId::new("qol-fixture"),
        manifest,
        plugin_root.clone(),
    );
    host.plugins.lock().unwrap().insert_plugin_for_test(plugin);
    let grant = OperationKey::new(
        crate::plugins::manifest::PluginUid::new("fixture-uid"),
        OperationKind::Action,
        "run",
    );
    for (metadata, accepted) in [
        ("agent_tool = true", false),
        ("peer = { replay = \"never\" }", true),
        ("", true),
    ] {
        fs::write(
            plugin_root.join("qol-runtime.toml"),
            format!("schema_version = 1\n[action.run]\ndescription = \"Run\"\n{metadata}\n"),
        )
        .unwrap();
        let expected = authority(&shared).expected();
        let reply = shared.peer_admin(Request::SetGrants {
            expected,
            peer_id: peer,
            grants: vec![grant.clone()],
        });
        assert_eq!(
            matches!(reply, Response::Changed { .. }),
            accepted,
            "{metadata}: {reply:?}"
        );
        if !accepted {
            assert_eq!(
                reply,
                Response::Error {
                    error: Error::GrantUnavailable
                }
            );
        }
    }
    fs::write(
        plugin_root.join("qol-runtime.toml"),
        "schema_version = 1\n[action.run]\ndescription = \"Run\"\npeer = { replay = \"never\" }\n",
    )
    .unwrap();
    for invalid in [
        OperationKey::new(
            crate::plugins::manifest::PluginUid::new("Fixture"),
            OperationKind::Action,
            "run",
        ),
        OperationKey::new(
            crate::plugins::manifest::PluginUid::new("fixture-uid"),
            OperationKind::Query,
            "run",
        ),
        OperationKey::new(
            crate::plugins::manifest::PluginUid::new("fixture-uid"),
            OperationKind::Action,
            "other",
        ),
        OperationKey {
            identity: qol_plugin_api::operations::OperationIdentity::Local(
                crate::plugins::manifest::PluginId::new("qol-fixture"),
            ),
            kind: OperationKind::Action,
            name: "run".into(),
        },
    ] {
        let reply = shared.peer_admin(Request::SetGrants {
            expected: authority(&shared).expected(),
            peer_id: peer,
            grants: vec![invalid],
        });
        assert_eq!(
            reply,
            Response::Error {
                error: Error::GrantUnavailable
            }
        );
    }
    *host.plugins.lock().unwrap() = PluginManager::new();
    assert!(matches!(
        shared.peer_admin(Request::SetGrants {
            expected: authority(&shared).expected(),
            peer_id: peer,
            grants: vec![]
        }),
        Response::Changed { .. }
    ));
    assert_eq!(authority(&shared).grant_count, 0);
}

fn assert_reopened(reopened: &AuthoritySummary, previous: &AuthoritySummary) {
    assert_ne!(reopened.activation_id, previous.activation_id);
    let mut previous = previous.clone();
    previous.activation_id = reopened.activation_id;
    assert_eq!(*reopened, previous);
}

#[test]
fn reducing_grants_preserves_unavailable_operations_but_cannot_add_another() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peer = populated(&root, 1)[0];
    let host = host_at(&root, false);
    let shared = attach(&host);
    let expected = authority(&shared).expected();
    let grants = host
        .inner
        .lock()
        .unwrap()
        .authority()
        .unwrap()
        .authority
        .projection()
        .unwrap()
        .peers[0]
        .grants
        .clone();
    let preserved = grants[1..].to_vec();
    assert!(matches!(
        shared.peer_admin(Request::SetGrants {
            expected,
            peer_id: peer,
            grants: preserved.clone()
        }),
        Response::Changed { .. }
    ));
    let actual = host
        .inner
        .lock()
        .unwrap()
        .authority()
        .unwrap()
        .authority
        .projection()
        .unwrap()
        .peers[0]
        .grants
        .clone();
    assert_eq!(actual, preserved);
    assert_eq!(
        shared.peer_admin(Request::SetGrants {
            expected: authority(&shared).expected(),
            peer_id: peer,
            grants
        }),
        Response::Error {
            error: Error::GrantUnavailable
        }
    );
    host.shutdown();
}
