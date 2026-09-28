use crate::features::linked_computers::settings::{CatalogOperation, InvitationInfo, Snapshot};
use qol_gpui::settings_panel::SettingsValueTone;
use qol_peers::admin::{
    AuthoritySummary, EnrollmentRequest, ExpectedAuthority, Lifecycle, PointzRequest, PointzStatus,
    Request,
};
use qol_peers::enrollment::{ExportedInvitation, OutboundEnrollmentState};
use qol_peers::pointz::{PointzDevice, PointzPlugin, PointzTransport};
use qol_peers::{AuthorityLifetime, AuthorityStatus};

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

pub(super) const NAME_RULE: &str =
    "Give this computer a name first. It cannot be empty, start or end with a space, or run past 256 bytes.";

#[derive(Clone)]
pub(super) enum Action {
    Refresh,
    Name,
    Paste,
    Copy,
    Explain(&'static str),
    Send(Request, Option<&'static str>),
}

pub(super) struct Row {
    pub label: String,
    pub detail: String,
    pub value: Option<(String, SettingsValueTone)>,
    pub verb: Option<&'static str>,
    pub action: Option<Action>,
    pub header: bool,
}

impl Row {
    fn new(label: impl Into<String>, detail: impl Into<String>, action: Option<Action>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            value: None,
            verb: None,
            action,
            header: false,
        }
    }

    fn header(title: &str, colophon: &str) -> Self {
        Self {
            header: true,
            ..Self::new(title, colophon, None)
        }
    }

    fn value(mut self, text: impl Into<String>, tone: SettingsValueTone) -> Self {
        self.value = Some((text.into(), tone));
        self
    }

    fn verb(mut self, verb: &'static str) -> Self {
        self.verb = Some(verb);
        self
    }
}

pub(super) fn rows(
    snapshot: Option<&Snapshot>,
    catalog: Option<&[CatalogOperation]>,
    name: &str,
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    invitation: Option<&qol_peers::admin::Response>,
) -> Vec<Row> {
    let named = qol_peers::is_valid_name(name);
    let mut rows = vec![
        Row::header("this computer", "how other computers see this one"),
        Row::new(
            "Computer name",
            if named {
                "Shown to the computers you link"
            } else {
                NAME_RULE
            },
            Some(Action::Name),
        )
        .value(
            if name.is_empty() { "not set" } else { name },
            if named {
                SettingsValueTone::Normal
            } else {
                SettingsValueTone::Danger
            },
        )
        .verb("edit"),
        Row::new("Refresh", "Read linking state again", Some(Action::Refresh)).verb("refresh"),
    ];
    let Some(snapshot) = snapshot else {
        return rows;
    };
    let Some(authority) = &snapshot.status.authority else {
        rows.push(
            Row::new(
                "Linking",
                "Choose how long links belong to this computer",
                None,
            )
            .value(
                lifecycle_label(&snapshot.status.lifecycle),
                SettingsValueTone::Muted,
            ),
        );
        if matches!(
            snapshot.status.lifecycle,
            Lifecycle::Inactive | Lifecycle::Unavailable { .. }
        ) {
            rows.push(
                named_send(
                    "Use this session",
                    "Links end when this computer's session stops",
                    named,
                    Request::StartSession { name: name.into() },
                    None,
                )
                .verb("start"),
            );
            rows.push(
                named_send(
                    "Create persistent links",
                    "Kept on this computer; needs platform support",
                    named,
                    Request::CreatePersistent { name: name.into() },
                    Some("persist"),
                )
                .verb("create"),
            );
            rows.push(
                send(
                    "Open persistent links",
                    "Use this computer's existing links",
                    Request::OpenPersistent,
                    None,
                )
                .verb("open"),
            );
        }
        return rows;
    };
    let expected = authority.expected();
    let ready = matches!(snapshot.status.lifecycle, Lifecycle::Active)
        && authority.status == AuthorityStatus::Ready;
    rows.push(
        Row::new(
            "Linking",
            format!("{} \u{b7} {}", authority.name, authority.peer_id),
            None,
        )
        .value(
            linking_label(&snapshot.status.lifecycle, authority),
            if ready {
                SettingsValueTone::Success
            } else {
                SettingsValueTone::Attention
            },
        ),
    );
    if !ready {
        rows.push(
            Row::new(
                "Service unavailable",
                "Refresh to read it again before changing anything",
                None,
            )
            .value(
                lifecycle_label(&snapshot.status.lifecycle),
                SettingsValueTone::Attention,
            ),
        );
        return rows;
    }
    rows.push(
        named_send(
            "Rename",
            "Use the name above for this computer",
            named,
            Request::Rename {
                expected,
                name: name.into(),
            },
            None,
        )
        .verb("rename"),
    );
    rows.push(
        send(
            "Stop linking",
            "Stop before changing how long links last",
            Request::Stop { expected },
            Some("stop"),
        )
        .verb("stop"),
    );

    rows.push(Row::header(
        "invitations",
        "link another computer to this one",
    ));
    rows.push(
        enroll(
            "Create invitation",
            "Make a one-time code for the other computer",
            EnrollmentRequest::CreateInvitation {
                expected,
                addresses: Vec::new(),
            },
            None,
        )
        .verb("create"),
    );
    if let Some(qol_peers::admin::Response::Invitation {
        authority,
        invitation,
        ..
    }) = invitation
    {
        if authority.authority_id == expected.authority_id
            && authority.activation_id == expected.activation_id
        {
            rows.push(
                Row::new(
                    "Copy invitation",
                    "Copy only to the computer you mean to link",
                    Some(Action::Copy),
                )
                .value("ready", SettingsValueTone::Success)
                .verb("copy"),
            );
            rows.push(
                enroll(
                    "Cancel invitation",
                    "The code stops working",
                    EnrollmentRequest::CancelInvitation {
                        expected,
                        invitation: *invitation,
                    },
                    None,
                )
                .verb("cancel"),
            );
        }
    }
    rows.push(
        Row::new(
            "Paste invitation",
            "Read a code copied on the other computer",
            Some(Action::Paste),
        )
        .verb("paste"),
    );
    if let Some((document, info)) = source {
        if !snapshot
            .outbound
            .iter()
            .any(|item| item.key.invitation == info.invitation)
        {
            rows.push(
                enroll(
                    "Prepare link",
                    &format!("Invitation from {}", info.peer),
                    EnrollmentRequest::Prepare {
                        expected,
                        document: document.clone(),
                    },
                    None,
                )
                .verb("prepare"),
            );
        }
    }

    if !snapshot.pending.is_empty() || !snapshot.outbound.is_empty() {
        rows.push(Row::header("requests", "links waiting on a decision"));
    }
    for pending in &snapshot.pending {
        let detail = format!(
            "{} \u{b7} {} \u{b7} {}",
            pending.name,
            pending.key.peer,
            lifetime_label(pending.remote_lifetime)
        );
        rows.push(
            enroll(
                "Approve request",
                &detail,
                EnrollmentRequest::Approve {
                    expected,
                    key: pending.key,
                },
                Some("approve"),
            )
            .verb("approve"),
        );
        rows.push(
            enroll(
                "Reject request",
                &detail,
                EnrollmentRequest::Reject {
                    expected,
                    key: pending.key,
                },
                None,
            )
            .verb("reject"),
        );
    }
    for item in &snapshot.outbound {
        rows.extend(outbound_rows(item, snapshot, source, expected));
    }

    rows.push(Row::header(
        "linked computers",
        "computers that can reach this one",
    ));
    if snapshot.peers.is_empty() {
        rows.push(Row::new(
            "No linked computers yet",
            "Create an invitation to link one",
            None,
        ));
    }
    for peer in &snapshot.peers {
        let connected = snapshot
            .sessions
            .iter()
            .any(|session| session.peer_id == peer.peer_id);
        rows.push(
            Row::new(
                peer.name.clone(),
                format!(
                    "{} \u{b7} {} at last refresh",
                    peer.peer_id,
                    if connected {
                        "Authenticated connection"
                    } else {
                        "Not connected"
                    }
                ),
                None,
            )
            .value(
                if connected {
                    "connected"
                } else {
                    "not connected"
                },
                if connected {
                    SettingsValueTone::Success
                } else {
                    SettingsValueTone::Muted
                },
            ),
        );
        rows.push(
            send(
                "Revoke link",
                &peer.name,
                Request::Revoke {
                    expected,
                    peer_id: peer.peer_id,
                },
                Some("revoke"),
            )
            .verb("revoke"),
        );
        if let Some((_, grants)) = snapshot.grants.iter().find(|(id, _)| *id == peer.peer_id) {
            for grant in grants {
                rows.push(
                    send(
                        "Remove permission",
                        &format!("{} \u{b7} {:?}", peer.name, grant),
                        Request::SetGrants {
                            expected,
                            peer_id: peer.peer_id,
                            grants: grants.iter().filter(|key| *key != grant).cloned().collect(),
                        },
                        Some("remove"),
                    )
                    .value("allowed", SettingsValueTone::Success)
                    .verb("remove"),
                );
            }
            for operation in catalog.into_iter().flatten() {
                if grants.contains(&operation.key) {
                    continue;
                }
                let mut updated = grants.clone();
                updated.push(operation.key.clone());
                rows.push(
                    send(
                        "Add permission",
                        &format!(
                            "{} \u{b7} {} \u{b7} {} ({} {})",
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
                    )
                    .value("not allowed", SettingsValueTone::Muted)
                    .verb("add"),
                );
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
    if let Some(pointz) = &snapshot.pointz {
        rows.extend(phone_rows(pointz, &snapshot.phones, expected));
    }
    rows
}

fn outbound_rows(
    item: &qol_peers::enrollment::OutboundEnrollment,
    snapshot: &Snapshot,
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let transaction = item.key.transaction;
    let attempt = snapshot
        .attempts
        .iter()
        .find(|(id, _)| *id == transaction)
        .map(|(_, state)| state);
    let detail = format!(
        "{} \u{b7} {} \u{b7} {}",
        item.key.peer,
        transaction,
        attempt
            .map(crate::features::linked_computers::settings::attempt_label)
            .unwrap_or("unavailable")
    );
    let (state, tone) = match item.state {
        OutboundEnrollmentState::Pending {} => ("pending", SettingsValueTone::Attention),
        OutboundEnrollmentState::Abandoned {} => ("abandoned", SettingsValueTone::Muted),
        OutboundEnrollmentState::Committed { .. } => ("linked", SettingsValueTone::Success),
    };
    let mut rows = vec![Row::new("Outgoing request", detail.clone(), None).value(state, tone)];
    if matches!(item.state, OutboundEnrollmentState::Pending {}) {
        if let Some((document, info)) = source.filter(|(_, info)| {
            info.invitation == item.key.invitation && info.peer == item.key.peer
        }) {
            rows.push(
                enroll(
                    "Send prepared request",
                    &detail,
                    EnrollmentRequest::Redeem {
                        expected,
                        transaction,
                        document: document.clone(),
                    },
                    None,
                )
                .verb("send"),
            );
            rows.push(
                enroll(
                    "Recover original transaction",
                    &detail,
                    EnrollmentRequest::Recover {
                        expected,
                        transaction,
                        endpoints: info.endpoints.clone(),
                    },
                    None,
                )
                .verb("recover"),
            );
        }
        rows.push(
            enroll(
                "Abandon",
                &detail,
                EnrollmentRequest::Abandon {
                    expected,
                    transaction,
                },
                Some("abandon"),
            )
            .verb("abandon"),
        );
    }
    if matches!(item.state, OutboundEnrollmentState::Abandoned {}) {
        rows.push(
            enroll(
                "Resume original transaction",
                &detail,
                EnrollmentRequest::Resume {
                    expected,
                    transaction,
                },
                None,
            )
            .verb("resume"),
        );
    }
    rows
}

fn phone_rows(
    status: &PointzStatus,
    phones: &[PointzDevice],
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = Vec::new();
    match status.plugin {
        PointzPlugin::Absent => return rows,
        PointzPlugin::Legacy => {
            rows.push(Row::header(
                "phones",
                "pointz phones that reach this computer",
            ));
            rows.push(Row::new(
                "PointZ",
                "Keeps its own phone pairing until PointZ is updated",
                None,
            ));
            return rows;
        }
        PointzPlugin::Compatible => {}
    }
    rows.push(Row::header(
        "phones",
        "pointz phones that reach this computer",
    ));
    if status
        .migration
        .is_some_and(|migration| migration.phones_must_pair_again())
    {
        rows.push(
            Row::new(
                "Some phones must pair again",
                "Not every PointZ pairing could move into linked computers",
                None,
            )
            .value("pair again", SettingsValueTone::Attention),
        );
    }
    if matches!(
        status.transport,
        PointzTransport::PortBusy { .. } | PointzTransport::Failed { .. }
    ) {
        rows.push(
            Row::new(
                "PointZ is not listening",
                "Another program may be using its network ports",
                None,
            )
            .value("not listening", SettingsValueTone::Attention),
        );
    }
    if phones.is_empty() {
        rows.push(Row::new(
            "No paired phones",
            "Pair a phone from PointZ settings",
            None,
        ));
    }
    for phone in phones {
        rows.push(
            send(
                "Remove phone",
                &phone.name,
                Request::Pointz {
                    request: PointzRequest::Remove {
                        expected,
                        device_id: phone.device_id,
                    },
                },
                Some("remove-phone"),
            )
            .verb("remove"),
        );
    }
    rows
}

fn lifecycle_label(lifecycle: &Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Inactive => "off",
        Lifecycle::Standby => "starting",
        Lifecycle::Active => "on",
        Lifecycle::Stopping => "stopping",
        Lifecycle::Unavailable { .. } => "unavailable",
        Lifecycle::Shutdown => "shut down",
    }
}

fn lifetime_label(lifetime: AuthorityLifetime) -> &'static str {
    match lifetime {
        AuthorityLifetime::Session => "this session",
        AuthorityLifetime::Persistent => "persistent",
    }
}

fn linking_label(lifecycle: &Lifecycle, authority: &AuthoritySummary) -> String {
    if authority.status != AuthorityStatus::Ready {
        return "faulted".into();
    }
    match (lifecycle, authority.lifetime) {
        (Lifecycle::Active, AuthorityLifetime::Session) => "on for this session".into(),
        (Lifecycle::Active, AuthorityLifetime::Persistent) => "on, persistent".into(),
        (lifecycle, _) => lifecycle_label(lifecycle).into(),
    }
}

fn send(label: &str, detail: &str, request: Request, confirm: Option<&'static str>) -> Row {
    Row::new(label, detail, Some(Action::Send(request, confirm)))
}

fn named_send(
    label: &str,
    detail: &str,
    named: bool,
    request: Request,
    confirm: Option<&'static str>,
) -> Row {
    if named {
        return send(label, detail, request, confirm);
    }
    Row::new(label, detail, Some(Action::Explain(NAME_RULE)))
}

fn enroll(
    label: &str,
    detail: &str,
    request: EnrollmentRequest,
    confirm: Option<&'static str>,
) -> Row {
    send(label, detail, Request::Enrollment { request }, confirm)
}
