use crate::features::linked_computers::settings::{CatalogOperation, InvitationInfo, Snapshot};
use crate::plugins::PluginId;
use qol_gpui::settings_panel::SettingsValueTone;
use qol_peers::admin::{
    AuthoritySummary, EnrollmentFailure, EnrollmentRequest, ExpectedAuthority, Lifecycle,
    NearbyRequest, NearbyState, PointzRequest, PointzStatus, Request,
};
use qol_peers::enrollment::{EnrollmentRejection, ExportedInvitation, OutboundEnrollmentState};
use qol_peers::pointz::{PointzDevice, PointzPlugin, PointzTransport};
use qol_peers::{AuthorityLifetime, AuthorityStatus};

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

pub(super) const NAME_RULE: &str =
    "A name cannot be empty, start or end with a space, or run past 256 bytes.";

#[derive(Clone)]
pub(super) enum Action {
    Refresh,
    Name,
    Paste,
    Copy,
    Toggle(PluginId),
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
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    invitation: Option<&qol_peers::admin::Response>,
    withheld: &[PluginId],
) -> Vec<Row> {
    let mut rows = vec![Row::header(
        "this computer",
        "how other computers see this one",
    )];
    let Some(snapshot) = snapshot else {
        rows.push(
            Row::new("Refresh", "Read linking state again", Some(Action::Refresh)).verb("refresh"),
        );
        return rows;
    };
    let Some(authority) = &snapshot.status.authority else {
        let off = matches!(
            snapshot.status.lifecycle,
            Lifecycle::Inactive | Lifecycle::Unavailable { .. }
        );
        let mut row = Row::new(
            "Linking",
            match snapshot.status.lifecycle {
                Lifecycle::Unavailable { error } => format!("Linking could not start: {error}"),
                _ => "Links this computer to your other computers".into(),
            },
            off.then(|| Action::Send(Request::Enable, None)),
        )
        .value(
            lifecycle_label(&snapshot.status.lifecycle),
            SettingsValueTone::Muted,
        );
        if off {
            row = row.verb("turn on");
        }
        rows.push(row);
        return rows;
    };
    let expected = authority.expected();
    let ready = matches!(snapshot.status.lifecycle, Lifecycle::Active)
        && authority.status == AuthorityStatus::Ready;
    rows.push(
        Row::new(
            "Computer name",
            "Shown to the computers you link",
            ready.then_some(Action::Name),
        )
        .value(authority.name.clone(), SettingsValueTone::Normal)
        .verb("rename"),
    );
    let mut linking = Row::new("Linking", lifetime_detail(authority.lifetime), None).value(
        linking_label(&snapshot.status.lifecycle, authority),
        if ready {
            SettingsValueTone::Success
        } else {
            SettingsValueTone::Attention
        },
    );
    if ready {
        linking.action = Some(Action::Send(Request::Stop { expected }, Some("stop")));
        linking = linking.verb("stop");
    }
    rows.push(linking);
    if !ready {
        return rows;
    }

    rows.extend(nearby_rows(snapshot, catalog, withheld, expected));
    rows.extend(linked_rows(snapshot, catalog, expected));
    rows.extend(invitation_rows(snapshot, source, invitation, expected));
    if let Some(pointz) = &snapshot.pointz {
        rows.extend(phone_rows(pointz, &snapshot.phones, expected));
    }
    rows
}

fn nearby_rows(
    snapshot: &Snapshot,
    catalog: Option<&[CatalogOperation]>,
    withheld: &[PluginId],
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = vec![Row::header("nearby", "computers on this network")];
    if snapshot.nearby.is_empty() {
        rows.push(Row::new(
            "Looking for computers",
            "Open Linked computers on the other computer. Both need to be on this network.",
            None,
        ));
        return rows;
    }
    let grants: Vec<_> = catalog
        .into_iter()
        .flatten()
        .filter(|operation| !withheld.contains(&operation.plugin_id))
        .map(|operation| operation.key.clone())
        .collect();
    let mut confirming = false;
    for computer in &snapshot.nearby {
        let link = |peer_id| Request::Nearby {
            request: NearbyRequest::Link { expected, peer_id },
        };
        let decline = Request::Nearby {
            request: NearbyRequest::Decline {
                expected,
                peer_id: computer.peer_id,
            },
        };
        let Some(state) = &computer.link else {
            rows.push(
                send(
                    &computer.name,
                    "On this network, not linked",
                    link(computer.peer_id),
                    None,
                )
                .verb("link"),
            );
            continue;
        };
        let code = state.code.map(|code| code.to_string());
        match state.state {
            NearbyState::Connecting {} => rows.push(
                Row::new(&computer.name, "Asking it for a code", None)
                    .value("connecting", SettingsValueTone::Muted),
            ),
            NearbyState::Confirm {} => {
                confirming = true;
                rows.push(
                    send(
                        &computer.name,
                        &format!("Link if {} shows this code too", computer.name),
                        Request::Nearby {
                            request: NearbyRequest::Confirm {
                                expected,
                                peer_id: computer.peer_id,
                                grants: grants.clone(),
                            },
                        },
                        None,
                    )
                    .value(code.unwrap_or_default(), SettingsValueTone::Attention)
                    .verb("link"),
                );
                rows.push(
                    send(
                        "Decline",
                        &format!(
                            "The codes differ, or you did not ask to link {}",
                            computer.name
                        ),
                        decline,
                        None,
                    )
                    .verb("decline"),
                );
            }
            NearbyState::WaitingForPeer {} => rows.push(
                send(
                    &computer.name,
                    &format!("Now choose Link on {}", computer.name),
                    decline,
                    None,
                )
                .value(code.unwrap_or_default(), SettingsValueTone::Normal)
                .verb("cancel"),
            ),
            NearbyState::Failed { error } => rows.push(
                send(
                    &computer.name,
                    failure_detail(error),
                    link(computer.peer_id),
                    None,
                )
                .value("not linked", SettingsValueTone::Danger)
                .verb("retry"),
            ),
        }
    }
    if confirming {
        rows.extend(new_link_permissions(catalog, withheld));
    }
    rows
}

fn new_link_permissions(catalog: Option<&[CatalogOperation]>, withheld: &[PluginId]) -> Vec<Row> {
    let mut plugins: Vec<&CatalogOperation> = Vec::new();
    for operation in catalog.into_iter().flatten() {
        if !plugins
            .iter()
            .any(|seen| seen.plugin_id == operation.plugin_id)
        {
            plugins.push(operation);
        }
    }
    plugins
        .into_iter()
        .map(|operation| {
            let allowed = !withheld.contains(&operation.plugin_id);
            Row::new(
                format!("Let it use {}", plugin_label(&operation.plugin_id)),
                operation.description.clone(),
                Some(Action::Toggle(operation.plugin_id.clone())),
            )
            .value(
                if allowed { "allowed" } else { "not allowed" },
                if allowed {
                    SettingsValueTone::Success
                } else {
                    SettingsValueTone::Muted
                },
            )
            .verb("change")
        })
        .collect()
}

fn linked_rows(
    snapshot: &Snapshot,
    catalog: Option<&[CatalogOperation]>,
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = vec![Row::header(
        "linked computers",
        "computers that can reach this one",
    )];
    if snapshot.peers.is_empty() {
        rows.push(Row::new(
            "No linked computers yet",
            "Link one from the nearby list",
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
                if connected {
                    "Authenticated connection at last refresh"
                } else {
                    "Not connected at last refresh"
                },
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
                        &format!("{} \u{b7} {}", peer.name, grant_label(grant, catalog)),
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
                        &format!("{} \u{b7} {}", peer.name, operation.description),
                        Request::SetGrants {
                            expected,
                            peer_id: peer.peer_id,
                            grants: updated,
                        },
                        None,
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
    rows
}

fn invitation_rows(
    snapshot: &Snapshot,
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    invitation: Option<&qol_peers::admin::Response>,
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = vec![Row::header(
        "other networks",
        "when the computers cannot see each other",
    )];
    let current = invitation.filter(|response| {
        matches!(response, qol_peers::admin::Response::Invitation { authority, .. }
            if authority.authority_id == expected.authority_id
                && authority.activation_id == expected.activation_id)
    });
    match current {
        Some(qol_peers::admin::Response::Invitation { invitation, .. }) => {
            rows.push(
                Row::new(
                    "Copy invitation",
                    "Paste it on the other computer within two minutes",
                    Some(Action::Copy),
                )
                .value("ready", SettingsValueTone::Success)
                .verb("copy"),
            );
            rows.push(
                enroll(
                    "Cancel invitation",
                    "The invitation stops working",
                    EnrollmentRequest::CancelInvitation {
                        expected,
                        invitation: *invitation,
                    },
                    None,
                )
                .verb("cancel"),
            );
        }
        _ => rows.push(
            enroll(
                "Create invitation",
                "Make a one-time invitation to paste on the other computer",
                EnrollmentRequest::CreateInvitation {
                    expected,
                    addresses: Vec::new(),
                },
                None,
            )
            .verb("create"),
        ),
    }
    rows.push(
        Row::new(
            "Paste invitation",
            "Link using an invitation copied on the other computer",
            Some(Action::Paste),
        )
        .verb("paste"),
    );
    for pending in &snapshot.pending {
        let detail = format!(
            "{} \u{b7} {}",
            pending.name,
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
                None,
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
        if !matches!(item.state, OutboundEnrollmentState::Pending {})
            || snapshot
                .nearby
                .iter()
                .any(|computer| computer.peer_id == item.key.peer && computer.link.is_some())
        {
            continue;
        }
        rows.extend(outbound_rows(item, snapshot, source, expected));
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
        "Waiting on the other computer \u{b7} {}",
        attempt
            .map(crate::features::linked_computers::settings::attempt_label)
            .unwrap_or("unavailable")
    );
    let mut rows = vec![Row::new("Outgoing request", detail.clone(), None)
        .value("pending", SettingsValueTone::Attention)];
    if let Some((_, info)) = source
        .filter(|(_, info)| info.invitation == item.key.invitation && info.peer == item.key.peer)
    {
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

fn failure_detail(error: EnrollmentFailure) -> &'static str {
    match error {
        EnrollmentFailure::Transport | EnrollmentFailure::Unavailable => {
            "Could not reach it. Check that Linked computers is open there, then retry."
        }
        EnrollmentFailure::Rejected(EnrollmentRejection::Expired) => {
            "The code ran out before both computers chose Link"
        }
        EnrollmentFailure::Rejected(
            EnrollmentRejection::Cancelled | EnrollmentRejection::InvalidInvitation,
        ) => "It declined, or its request ended",
        EnrollmentFailure::Rejected(EnrollmentRejection::Revoked) => {
            "This computer revoked it before, so it cannot link again"
        }
        EnrollmentFailure::Rejected(EnrollmentRejection::Capacity)
        | EnrollmentFailure::Capacity => "It is busy with other requests. Retry in a moment.",
        EnrollmentFailure::Abandoned => "Cancelled here",
        _ => "Linking failed. Retry to start again.",
    }
}

fn plugin_label(plugin: &PluginId) -> String {
    let name = plugin
        .as_str()
        .strip_prefix("qol-")
        .unwrap_or(plugin.as_str());
    let mut characters = name.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

fn grant_label(
    grant: &qol_plugin_api::operations::OperationKey,
    catalog: Option<&[CatalogOperation]>,
) -> String {
    catalog
        .into_iter()
        .flatten()
        .find(|operation| operation.key == *grant)
        .map(|operation| operation.description.clone())
        .unwrap_or_else(|| format!("{} {}", grant.kind.as_str(), grant.name))
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

fn lifetime_detail(lifetime: AuthorityLifetime) -> &'static str {
    match lifetime {
        AuthorityLifetime::Session => "Links end when qol-tray stops on this computer",
        AuthorityLifetime::Persistent => "Links stay on this computer",
    }
}

fn linking_label(lifecycle: &Lifecycle, authority: &AuthoritySummary) -> String {
    if authority.status != AuthorityStatus::Ready {
        return "faulted".into();
    }
    match lifecycle {
        Lifecycle::Active => "on".into(),
        lifecycle => lifecycle_label(lifecycle).into(),
    }
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
