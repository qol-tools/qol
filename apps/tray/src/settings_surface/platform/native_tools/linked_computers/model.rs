use crate::features::linked_computers::settings::{CatalogOperation, InvitationInfo, Snapshot};
use qol_peers::admin::{EnrollmentRequest, Lifecycle, Request};
use qol_peers::enrollment::{ExportedInvitation, OutboundEnrollmentState};

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) enum Action {
    Refresh,
    Name,
    Paste,
    Copy,
    Send(Request, Option<&'static str>),
}

pub(super) struct Row {
    pub label: String,
    pub detail: String,
    pub action: Option<Action>,
}

impl Row {
    fn new(label: impl Into<String>, detail: impl Into<String>, action: Option<Action>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            action,
        }
    }
}

pub(super) fn rows(
    snapshot: Option<&Snapshot>,
    catalog: Option<&[CatalogOperation]>,
    name: &str,
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    invitation: Option<&qol_peers::admin::Response>,
) -> Vec<Row> {
    let mut rows = vec![
        Row::new("Refresh", "Read current core state", Some(Action::Refresh)),
        Row::new("Computer name", name, Some(Action::Name)),
    ];
    let Some(snapshot) = snapshot else {
        return rows;
    };
    let Some(authority) = &snapshot.status.authority else {
        rows.push(Row::new(
            "Service",
            format!("{:?}", snapshot.status.lifecycle),
            None,
        ));
        if matches!(
            snapshot.status.lifecycle,
            Lifecycle::Inactive | Lifecycle::Unavailable { .. }
        ) {
            rows.push(send(
                "Use this session",
                "Links end when this authority stops",
                Request::StartSession { name: name.into() },
                None,
            ));
            rows.push(send(
                "Create persistent links",
                "Requires platform support; retained on this computer",
                Request::CreatePersistent { name: name.into() },
                Some("persist"),
            ));
            rows.push(send(
                "Open persistent links",
                "Use this computer's existing authority",
                Request::OpenPersistent,
                None,
            ));
        }
        return rows;
    };
    let expected = authority.expected();
    rows.push(Row::new(
        authority.name.clone(),
        format!("{:?} · {}", authority.lifetime, authority.peer_id),
        None,
    ));
    if !matches!(snapshot.status.lifecycle, Lifecycle::Active)
        || authority.status != qol_peers::AuthorityStatus::Ready
    {
        rows.push(Row::new(
            "Service unavailable",
            format!("{:?} · {:?}", snapshot.status.lifecycle, authority.status),
            None,
        ));
        return rows;
    }
    rows.push(send(
        "Rename",
        "Apply the entered computer name",
        Request::Rename {
            expected,
            name: name.into(),
        },
        None,
    ));
    rows.push(send(
        "Stop linking",
        "Stop before changing lifetime",
        Request::Stop { expected },
        Some("stop"),
    ));
    rows.push(enroll(
        "Create invitation",
        "Select local addresses automatically",
        EnrollmentRequest::CreateInvitation {
            expected,
            addresses: Vec::new(),
        },
        None,
    ));
    if let Some(qol_peers::admin::Response::Invitation {
        authority,
        invitation,
        ..
    }) = invitation
    {
        if authority.authority_id == expected.authority_id
            && authority.activation_id == expected.activation_id
        {
            rows.push(Row::new(
                "Copy invitation",
                "Copy only to the intended computer",
                Some(Action::Copy),
            ));
            rows.push(enroll(
                "Cancel invitation",
                "Invalidate this invitation",
                EnrollmentRequest::CancelInvitation {
                    expected,
                    invitation: *invitation,
                },
                None,
            ));
        }
    }
    rows.push(Row::new(
        "Paste invitation",
        "Read an opaque qol-link code from the clipboard",
        Some(Action::Paste),
    ));
    if let Some((document, info)) = source {
        if !snapshot
            .outbound
            .iter()
            .any(|item| item.key.invitation == info.invitation)
        {
            rows.push(enroll(
                "Prepare link",
                &info.peer.to_string(),
                EnrollmentRequest::Prepare {
                    expected,
                    document: document.clone(),
                },
                None,
            ));
        }
    }
    for pending in &snapshot.pending {
        let detail = format!(
            "{} · {} · {:?}",
            pending.name, pending.key.peer, pending.remote_lifetime
        );
        rows.push(enroll(
            "Approve request",
            &detail,
            EnrollmentRequest::Approve {
                expected,
                key: pending.key,
            },
            Some("approve"),
        ));
        rows.push(enroll(
            "Reject request",
            &detail,
            EnrollmentRequest::Reject {
                expected,
                key: pending.key,
            },
            None,
        ));
    }
    for item in &snapshot.outbound {
        let transaction = item.key.transaction;
        let attempt = snapshot
            .attempts
            .iter()
            .find(|(id, _)| *id == transaction)
            .map(|(_, state)| state);
        let detail = format!(
            "{} · {} · {}",
            item.key.peer,
            transaction,
            attempt
                .map(crate::features::linked_computers::settings::attempt_label)
                .unwrap_or("unavailable")
        );
        rows.push(Row::new(
            format!(
                "Outgoing: {}",
                match item.state {
                    OutboundEnrollmentState::Pending {} => "pending",
                    OutboundEnrollmentState::Abandoned {} => "abandoned",
                    OutboundEnrollmentState::Committed { .. } => "committed",
                }
            ),
            detail.clone(),
            None,
        ));
        if matches!(item.state, OutboundEnrollmentState::Pending {}) {
            if let Some((document, info)) = source.filter(|(_, info)| {
                info.invitation == item.key.invitation && info.peer == item.key.peer
            }) {
                rows.push(enroll(
                    "Send prepared request",
                    &detail,
                    EnrollmentRequest::Redeem {
                        expected,
                        transaction,
                        document: document.clone(),
                    },
                    None,
                ));
                rows.push(enroll(
                    "Recover original transaction",
                    &detail,
                    EnrollmentRequest::Recover {
                        expected,
                        transaction,
                        endpoints: info.endpoints.clone(),
                    },
                    None,
                ));
            }
            rows.push(enroll(
                "Abandon",
                &detail,
                EnrollmentRequest::Abandon {
                    expected,
                    transaction,
                },
                Some("abandon"),
            ));
        }
        if matches!(item.state, OutboundEnrollmentState::Abandoned {}) {
            rows.push(enroll(
                "Resume original transaction",
                &detail,
                EnrollmentRequest::Resume {
                    expected,
                    transaction,
                },
                None,
            ));
        }
    }
    for peer in &snapshot.peers {
        let connected = snapshot
            .sessions
            .iter()
            .any(|session| session.peer_id == peer.peer_id);
        rows.push(Row::new(
            peer.name.clone(),
            format!(
                "{} · {} at last refresh",
                peer.peer_id,
                if connected {
                    "Authenticated connection"
                } else {
                    "Not connected"
                }
            ),
            None,
        ));
        rows.push(send(
            "Revoke link",
            &peer.name,
            Request::Revoke {
                expected,
                peer_id: peer.peer_id,
            },
            Some("revoke"),
        ));
        if let Some((_, grants)) = snapshot.grants.iter().find(|(id, _)| *id == peer.peer_id) {
            for grant in grants {
                rows.push(send(
                    "Remove permission",
                    &format!("{} · {:?}", peer.name, grant),
                    Request::SetGrants {
                        expected,
                        peer_id: peer.peer_id,
                        grants: grants.iter().filter(|key| *key != grant).cloned().collect(),
                    },
                    Some("remove"),
                ));
            }
            for operation in catalog.into_iter().flatten() {
                if grants.contains(&operation.key) {
                    continue;
                }
                let mut updated = grants.clone();
                updated.push(operation.key.clone());
                rows.push(send(
                    "Add permission",
                    &format!(
                        "{} · {} · {} ({} {})",
                        peer.name,
                        operation.plugin_id,
                        operation.description,
                        operation.key.kind.as_str(),
                        operation.key.name,
                    ),
                    Request::SetGrants {
                        expected,
                        peer_id: peer.peer_id,
                        grants: updated,
                    },
                    Some("add"),
                ));
            }
        }
    }
    if catalog.is_none() {
        rows.push(Row::new(
            "Permission catalog unavailable",
            "Refresh to retry. Existing permissions are retained.",
            None,
        ));
    }
    if catalog.is_some_and(|operations| operations.is_empty()) {
        rows.push(Row::new(
            "No available peer operations",
            "Existing permissions are retained.",
            None,
        ));
    }
    rows
}

fn send(label: &str, detail: &str, request: Request, confirm: Option<&'static str>) -> Row {
    Row::new(label, detail, Some(Action::Send(request, confirm)))
}

fn enroll(
    label: &str,
    detail: &str,
    request: EnrollmentRequest,
    confirm: Option<&'static str>,
) -> Row {
    send(label, detail, Request::Enrollment { request }, confirm)
}
