use super::{card, rows, Action, Control};
use crate::features::linked_devices::settings::{InvitationInfo, Snapshot};
use qol_peers::admin::{
    ActivationId, AuthoritySummary, EnrollmentRequest, Lifecycle, PeerSummary, Request, Status,
};
use qol_peers::enrollment::{
    EnrollmentRequestKey, ExportedInvitation, OutboundEnrollment, OutboundEnrollmentState,
};
use qol_peers::{AuthorityLifetime, AuthorityStatus, StoreRevision};
use qol_plugin_api::operations::{OperationKey, OperationKind};

fn snapshot() -> Snapshot {
    let peer_id = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        .parse()
        .unwrap();
    Snapshot {
        status: Status {
            lifecycle: Lifecycle::Active,
            authority: Some(AuthoritySummary {
                peer_id,
                activation_id: ActivationId::from_bytes([1; 16]),
                name: "local".into(),
                lifetime: AuthorityLifetime::Session,
                revision: StoreRevision::new(3),
                status: AuthorityStatus::Ready,
                peer_count: 0,
                grant_count: 0,
            }),
        },
        peers: Vec::new(),
        sessions: Vec::new(),
        grants: Vec::new(),
        pending: Vec::new(),
        outbound: Vec::new(),
        attempts: Vec::new(),
        pointz: None,
        phones: Vec::new(),
        nearby: Vec::new(),
    }
}

#[test]
fn confirmed_grant_removal_freezes_peer_and_stamp_and_retains_other_hidden_grants() {
    let mut snapshot = snapshot();
    let peer_id = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    let expected = snapshot.status.authority.as_ref().unwrap().expected();
    let grants: Vec<_> = ["hidden", "removed"]
        .into_iter()
        .map(|name| {
            OperationKey::new(
                crate::plugins::manifest::PluginUid::new("unavailable"),
                OperationKind::Action,
                name,
            )
        })
        .collect();
    snapshot.peers.push(PeerSummary {
        peer_id,
        name: "remote".into(),
        grant_count: 2,
    });
    snapshot.grants.push((peer_id, grants.clone()));
    let requests: Vec<_> = card(Some(&snapshot), None, &[], peer_id)
        .unwrap()
        .1
        .into_iter()
        .filter(|row| row.verb == Some("remove"))
        .filter_map(|row| match row.action {
            Some(Action::Send(request @ Request::SetGrants { .. })) => Some(request),
            _ => None,
        })
        .collect();
    snapshot.status.authority.as_mut().unwrap().activation_id = ActivationId::from_bytes([2; 16]);
    assert_eq!(
        requests,
        vec![
            Request::SetGrants {
                expected,
                peer_id,
                grants: vec![grants[1].clone()]
            },
            Request::SetGrants {
                expected,
                peer_id,
                grants: vec![grants[0].clone()]
            },
        ]
    );
    assert_eq!(snapshot.grants[0].1, grants);
}

#[test]
fn recovery_uses_the_original_transaction_and_only_its_matching_invitation_endpoints() {
    let mut snapshot = snapshot();
    let expected = snapshot.status.authority.as_ref().unwrap().expected();
    let peer = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    let invitation = "AQEBAQEBAQEBAQEBAQEBAQ".parse().unwrap();
    let transaction = "AgICAgICAgICAgICAgICAg".parse().unwrap();
    snapshot.outbound.push(OutboundEnrollment {
        key: EnrollmentRequestKey {
            invitation,
            transaction,
            peer,
        },
        state: OutboundEnrollmentState::Pending {},
    });
    for matches in [false, true] {
        let endpoints = vec!["192.168.1.4:1234".parse().unwrap()];
        let source = (
            ExportedInvitation::from_owned(zeroize::Zeroizing::new("qol-link:fixture".into()))
                .unwrap(),
            InvitationInfo {
                invitation,
                peer: if matches { peer } else { expected.authority_id },
                endpoints: endpoints.clone(),
            },
        );
        let requests: Vec<_> = rows(Some(&snapshot), Some(&source), None, None, &[])
            .into_iter()
            .filter_map(|row| match row.action {
                Some(Action::Send(Request::Enrollment { request })) => Some(request),
                _ => None,
            })
            .collect();
        assert!(!requests
            .iter()
            .any(|request| matches!(request, EnrollmentRequest::Prepare { .. })));
        assert_eq!(
            requests.contains(&EnrollmentRequest::Recover {
                expected,
                transaction,
                endpoints
            }),
            matches
        );
    }
}

#[test]
fn unavailable_views_have_no_mutating_controls_and_connection_labels_require_sessions() {
    let mut snapshot = snapshot();
    let peer_id = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    snapshot.peers.push(PeerSummary {
        peer_id,
        name: "remote".into(),
        grant_count: 0,
    });
    assert!(rows(Some(&snapshot), None, None, None, &[])
        .iter()
        .any(|row| row.detail.contains("Not connected at last refresh")));
    let nonce = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    snapshot.sessions.push(qol_peers::admin::SessionSummary {
        peer_id,
        name: "remote".into(),
        generation: qol_peers::session::SessionGeneration {
            local: nonce,
            remote: nonce,
        },
    });
    assert!(rows(Some(&snapshot), None, None, None, &[])
        .iter()
        .any(|row| row
            .detail
            .contains("Authenticated connection at last refresh")));
    for lifecycle in [Lifecycle::Stopping, Lifecycle::Standby] {
        snapshot.status.lifecycle = lifecycle;
        assert!(!rows(Some(&snapshot), None, None, None, &[])
            .iter()
            .any(|row| matches!(row.action, Some(Action::Send(_)))));
        assert!(card(Some(&snapshot), None, &[], peer_id).is_none());
    }
}

#[test]
fn permission_additions_preserve_unavailable_grants_and_freeze_the_displayed_authority() {
    use crate::features::linked_devices::settings::CatalogOperation;
    let mut snapshot = snapshot();
    let peer_id = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    let expected = snapshot.status.authority.as_ref().unwrap().expected();
    let keys: Vec<_> = ["hidden", "granted", "new"]
        .into_iter()
        .map(|name| {
            OperationKey::new(
                crate::plugins::manifest::PluginUid::new("fixture"),
                OperationKind::Action,
                name,
            )
        })
        .collect();
    snapshot.peers.push(PeerSummary {
        peer_id,
        name: "remote".into(),
        grant_count: 2,
    });
    snapshot.grants.push((peer_id, keys[..2].to_vec()));
    let catalog: Vec<_> = keys[1..]
        .iter()
        .map(|key| CatalogOperation {
            key: key.clone(),
            plugin_id: crate::plugins::PluginId::new("fixture"),
            description: key.name.clone(),
        })
        .collect();
    let presented = card(Some(&snapshot), Some(&catalog), &[], peer_id)
        .unwrap()
        .1;
    let requests: Vec<_> = presented
        .into_iter()
        .filter(|row| row.control == Control::Toggle(false))
        .filter_map(|row| match row.action {
            Some(Action::Send(request @ Request::SetGrants { .. })) => Some(request),
            _ => None,
        })
        .collect();
    snapshot.status.authority.as_mut().unwrap().activation_id = ActivationId::from_bytes([2; 16]);
    assert_eq!(
        requests,
        vec![Request::SetGrants {
            expected,
            peer_id,
            grants: keys.clone()
        }]
    );
    assert_eq!(snapshot.grants[0].1, keys[..2]);
    for catalog in [None, Some([].as_slice())] {
        let presented = card(Some(&snapshot), catalog, &[], peer_id).unwrap().1;
        assert!(!presented
            .iter()
            .any(|row| matches!(row.control, Control::Toggle(_))
                && matches!(row.action, Some(Action::Send(Request::SetGrants { .. })))));
        assert_eq!(
            presented
                .iter()
                .filter(|row| row.verb == Some("remove"))
                .count(),
            2
        );
        assert_eq!(
            presented
                .iter()
                .any(|row| row.label == "Permission catalog unavailable"),
            catalog.is_none()
        );
        assert_eq!(
            presented
                .iter()
                .any(|row| row.label == "No available peer operations"),
            catalog.is_some()
        );
    }
}

#[test]
fn phone_removal_uses_the_displayed_stamp_and_legacy_pointz_offers_no_controls() {
    use qol_peers::admin::{PointzRequest, PointzStatus};
    use qol_peers::pointz::{
        PointzDevice, PointzDeviceId, PointzImport, PointzImportSource, PointzPairing,
        PointzPlugin, PointzTransport,
    };

    let mut snapshot = snapshot();
    let expected = snapshot.status.authority.as_ref().unwrap().expected();
    let device_id = PointzDeviceId::from_bytes([4; 16]);
    snapshot.pointz = Some(PointzStatus {
        plugin: PointzPlugin::Compatible,
        authority: Some(expected),
        migration: Some(PointzImport {
            source: PointzImportSource::Legacy,
            imported: 1,
            dropped: 1,
            seed_replaced: false,
            devices_unreadable: false,
        }),
        server_id: Some("server".into()),
        device_count: 1,
        pairing: PointzPairing::CLOSED,
        transport: PointzTransport::Running { dropped: 0 },
    });
    snapshot.phones.push(PointzDevice {
        device_id,
        name: "Pixel".into(),
        paired_at_ms: 1,
    });

    let shown = rows(Some(&snapshot), None, None, None, &[]);

    assert!(shown
        .iter()
        .any(|row| row.label == "Some phones must pair again"));
    let remove = shown
        .iter()
        .find(|row| row.label == "Remove phone")
        .expect("paired phone must be removable");
    assert_eq!(remove.detail, "Pixel");
    assert_eq!(remove.verb, Some("remove"));
    let Some(Action::Send(request)) = &remove.action else {
        panic!("a paired phone must be removable");
    };
    assert_eq!(
        *request,
        Request::Pointz {
            request: PointzRequest::Remove {
                expected,
                device_id,
            },
        }
    );

    snapshot.pointz.as_mut().unwrap().plugin = PointzPlugin::Legacy;
    let legacy = rows(Some(&snapshot), None, None, None, &[]);
    assert!(!legacy.iter().any(|row| row.label == "Remove phone"));
    assert!(legacy.iter().any(|row| row.label == "PointZ"));
}

#[test]
fn linking_that_is_off_offers_to_turn_it_on() {
    let mut snapshot = snapshot();
    snapshot.status.authority = None;
    snapshot.status.lifecycle = Lifecycle::Inactive;
    let shown = rows(Some(&snapshot), None, None, None, &[]);
    let row = shown.iter().find(|row| row.label == "Linking").unwrap();
    assert!(matches!(row.action, Some(Action::Send(Request::Enable))));
    assert_eq!(row.control, Control::Toggle(false));
}

#[test]
fn a_nearby_code_confirms_with_every_plugin_the_user_left_allowed() {
    use crate::features::linked_devices::settings::CatalogOperation;
    use crate::plugins::PluginId;
    use qol_peers::admin::{LinkCode, NearbyDevice, NearbyLink, NearbyRequest, NearbyState};

    let mut snapshot = snapshot();
    let expected = snapshot.status.authority.as_ref().unwrap().expected();
    let peer_id = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    let catalog: Vec<_> = ["qol-bluetooth", "qol-media"]
        .into_iter()
        .map(|plugin| CatalogOperation {
            key: OperationKey::new(
                crate::plugins::manifest::PluginUid::new(plugin),
                OperationKind::Action,
                "run",
            ),
            plugin_id: PluginId::new(plugin),
            description: plugin.into(),
        })
        .collect();
    snapshot.nearby.push(NearbyDevice {
        peer_id,
        name: "Desk".into(),
        link: None,
    });
    let shown = rows(Some(&snapshot), None, None, None, &[]);
    let desk = shown.iter().find(|row| row.label == "Desk").unwrap();
    assert!(matches!(
        &desk.action,
        Some(Action::Send(Request::Nearby { request: NearbyRequest::Link { peer_id: id, .. } })) if *id == peer_id
    ));
    assert!(!shown
        .iter()
        .any(|row| matches!(row.action, Some(Action::Toggle(_)))));

    snapshot.nearby[0].link = Some(NearbyLink {
        code: LinkCode::new(42_917),
        state: NearbyState::Confirm {},
    });
    let withheld = [PluginId::new("qol-media")];
    let shown = rows(Some(&snapshot), None, None, None, &[]);
    let desk = shown
        .iter()
        .find(|row| row.label == "Desk \u{b7} 042 917")
        .unwrap();
    assert!(matches!(desk.action, Some(Action::Open(id)) if id == peer_id));
    assert!(matches!(
        &desk.remove,
        Some((
            "decline",
            Action::Send(Request::Nearby {
                request: NearbyRequest::Decline { .. }
            })
        ))
    ));
    assert!(!shown
        .iter()
        .any(|row| matches!(row.action, Some(Action::Toggle(_)))));
    let (title, opened) = card(Some(&snapshot), Some(&catalog), &withheld, peer_id).unwrap();
    assert_eq!(title, "Desk");
    let link = &opened[0];
    assert_eq!(link.control, Control::Chip);
    let Some(Action::Send(request)) = &link.action else {
        panic!("a shown code must be confirmable");
    };
    assert_eq!(
        *request,
        Request::Nearby {
            request: NearbyRequest::Confirm {
                expected,
                peer_id,
                grants: vec![catalog[0].key.clone()],
            },
        }
    );
    let toggles: Vec<_> = opened
        .iter()
        .filter(|row| matches!(row.action, Some(Action::Toggle(_))))
        .map(|row| (row.label.as_str(), row.control.clone()))
        .collect();
    assert_eq!(
        toggles,
        [
            ("Let it use Bluetooth", Control::Toggle(true)),
            ("Let it use Media", Control::Toggle(false))
        ]
    );
    assert!(matches!(
        &opened.last().unwrap().action,
        Some(Action::Send(Request::Nearby {
            request: NearbyRequest::Decline { .. }
        }))
    ));
    let headers: Vec<_> = shown
        .iter()
        .filter(|row| row.header)
        .map(|row| row.label.as_str())
        .collect();
    assert!(headers.contains(&"link requests"));
    assert!(!headers.contains(&"nearby"));
    assert!(!shown.iter().any(|row| row.label == "Desk"));
}

#[test]
fn every_group_opens_with_a_header_and_headers_carry_no_action() {
    let shown = rows(Some(&snapshot()), None, None, None, &[]);
    assert!(shown[0].header);
    assert!(shown
        .iter()
        .filter(|row| row.header)
        .all(|row| row.action.is_none() && !row.detail.is_empty()));
    let titles: Vec<_> = shown
        .iter()
        .filter(|row| row.header)
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(
        titles,
        ["this device", "nearby", "linked devices", "other networks"]
    );
}

#[test]
fn a_linked_device_is_one_row_whose_card_holds_its_permissions() {
    use crate::features::linked_devices::settings::CatalogOperation;
    let mut snapshot = snapshot();
    let peer_id = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"
        .parse()
        .unwrap();
    snapshot.peers.push(PeerSummary {
        peer_id,
        name: "remote".into(),
        grant_count: 0,
    });
    let catalog = [CatalogOperation {
        key: OperationKey::new(
            crate::plugins::manifest::PluginUid::new("qol-bluetooth"),
            OperationKind::Action,
            "take_over",
        ),
        plugin_id: crate::plugins::PluginId::new("qol-bluetooth"),
        description: "Moves devices".into(),
    }];
    let shown = rows(Some(&snapshot), None, None, None, &[]);
    assert!(!shown
        .iter()
        .any(|row| matches!(row.control, Control::Toggle(_)) && row.label != "Linking"));
    let remote = shown.iter().find(|row| row.label == "remote").unwrap();
    assert!(matches!(remote.action, Some(Action::Open(id)) if id == peer_id));
    assert!(matches!(
        remote.remove,
        Some(("unlink", Action::Send(Request::Revoke { .. })))
    ));
    let (title, opened) = card(Some(&snapshot), Some(&catalog), &[], peer_id).unwrap();
    assert_eq!(title, "remote");
    let labels: Vec<_> = opened.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, ["Can use Bluetooth", "Unlink"]);
}
