use crate::features::linked_devices::settings::{CatalogOperation, InvitationInfo, Snapshot};
use crate::plugins::PluginId;
use qol_gpui::settings_panel::SettingsValueTone;
use qol_peers::admin::{
    AuthoritySummary, EnrollmentFailure, EnrollmentRequest, ExpectedAuthority, Lifecycle,
    NearbyRequest, NearbyState, PointzRequest, PointzStatus, Request,
};
use qol_peers::enrollment::{EnrollmentRejection, ExportedInvitation, OutboundEnrollmentState};
use qol_peers::pointz::{PointzDevice, PointzPlugin, PointzTransport};
use qol_peers::{AuthorityLifetime, AuthorityStatus, PeerId};
use qol_plugin_api::operations::OperationKey;

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
    Open(PeerId),
    Send(Request),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Control {
    None,
    Value(String, SettingsValueTone),
    Toggle(bool),
    Chip,
}

pub(super) struct Row {
    pub label: String,
    pub detail: String,
    pub control: Control,
    pub verb: Option<&'static str>,
    pub action: Option<Action>,
    pub remove: Option<(&'static str, Action)>,
    pub header: bool,
}

impl Row {
    fn new(label: impl Into<String>, detail: impl Into<String>, action: Option<Action>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            control: Control::None,
            verb: None,
            action,
            remove: None,
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
        self.control = Control::Value(text.into(), tone);
        self
    }

    fn toggle(mut self, on: bool) -> Self {
        self.control = Control::Toggle(on);
        self.verb = Some(if on { "turn off" } else { "turn on" });
        self
    }

    fn chip(mut self, verb: &'static str) -> Self {
        self.control = Control::Chip;
        self.verb = Some(verb);
        self
    }

    fn verb(mut self, verb: &'static str) -> Self {
        self.verb = Some(verb);
        self
    }

    fn remove(mut self, verb: &'static str, action: Action) -> Self {
        self.remove = Some((verb, action));
        self
    }
}

pub(super) fn rows(
    snapshot: Option<&Snapshot>,
    source: Option<&(ExportedInvitation, InvitationInfo)>,
    invitation: Option<&qol_peers::admin::Response>,
    catalog: Option<&[CatalogOperation]>,
    withheld: &[PluginId],
) -> Vec<Row> {
    let mut rows = vec![Row::header("this device", "how other devices see this one")];
    let Some(snapshot) = snapshot else {
        rows.push(
            Row::new("Refresh", "Read linking state again", Some(Action::Refresh)).chip("refresh"),
        );
        return rows;
    };
    let Some(authority) = &snapshot.status.authority else {
        let detail = match snapshot.status.lifecycle {
            Lifecycle::Unavailable { error } => format!("Linking could not start: {error}"),
            _ => "Links this device to your other devices".into(),
        };
        let row = match snapshot.status.lifecycle {
            Lifecycle::Inactive => {
                Row::new("Linking", detail, Some(Action::Send(Request::Enable))).toggle(false)
            }
            ref lifecycle => Row::new("Linking", detail, None)
                .value(lifecycle_label(lifecycle), SettingsValueTone::Muted),
        };
        rows.push(row);
        return rows;
    };
    let expected = authority.expected();
    let ready = matches!(snapshot.status.lifecycle, Lifecycle::Active)
        && authority.status == AuthorityStatus::Ready;
    let mut name = Row::new(
        "Device name",
        "Shown to the devices you link",
        ready.then_some(Action::Name),
    )
    .value(authority.name.clone(), SettingsValueTone::Normal);
    if ready {
        name = name.verb("rename");
    }
    rows.push(name);
    rows.push(if ready {
        Row::new(
            "Linking",
            lifetime_detail(authority.lifetime),
            Some(Action::Send(Request::Stop { expected })),
        )
        .toggle(true)
    } else {
        Row::new("Linking", lifetime_detail(authority.lifetime), None).value(
            linking_label(&snapshot.status.lifecycle, authority),
            SettingsValueTone::Attention,
        )
    });
    if !ready {
        return rows;
    }

    rows.extend(request_rows(snapshot, expected));
    rows.extend(nearby_rows(snapshot, catalog, withheld, expected));
    rows.extend(linked_rows(snapshot, expected));
    rows.extend(invitation_rows(snapshot, source, invitation, expected));
    if let Some(pointz) = &snapshot.pointz {
        rows.extend(phone_rows(pointz, &snapshot.phones, expected));
    }
    rows
}

fn request_rows(snapshot: &Snapshot, expected: ExpectedAuthority) -> Vec<Row> {
    let mut rows = Vec::new();
    for device in &snapshot.nearby {
        let Some(state) = &device.link else {
            continue;
        };
        let decline = decline(device.peer_id, expected);
        match state.state {
            NearbyState::Confirm {} => rows.push(
                Row::new(
                    request_label(device),
                    format!("Link if {} shows the same code", device.name),
                    Some(Action::Open(device.peer_id)),
                )
                .verb("open")
                .remove("decline", decline),
            ),
            NearbyState::WaitingForPeer {} => rows.push(
                Row::new(
                    request_label(device),
                    format!("Now choose Link on {}", device.name),
                    None,
                )
                .value("waiting", SettingsValueTone::Muted)
                .remove("cancel", decline),
            ),
            NearbyState::Connecting {} | NearbyState::Failed { .. } => {}
        }
    }
    if rows.is_empty() {
        return rows;
    }
    rows.insert(
        0,
        Row::header("link requests", "compare the code on both devices"),
    );
    rows
}

fn request_label(device: &qol_peers::admin::NearbyDevice) -> String {
    format!("{} \u{b7} {}", device.name, link_code(device))
}

fn link_code(device: &qol_peers::admin::NearbyDevice) -> String {
    device
        .link
        .as_ref()
        .and_then(|link| link.code)
        .map(|code| code.to_string())
        .unwrap_or_default()
}

fn decline(peer_id: PeerId, expected: ExpectedAuthority) -> Action {
    Action::Send(Request::Nearby {
        request: NearbyRequest::Decline { expected, peer_id },
    })
}

pub(super) fn card(
    snapshot: Option<&Snapshot>,
    catalog: Option<&[CatalogOperation]>,
    withheld: &[PluginId],
    peer_id: PeerId,
) -> Option<(String, Vec<Row>)> {
    let snapshot = snapshot?;
    let authority = snapshot.status.authority.as_ref()?;
    if !matches!(snapshot.status.lifecycle, Lifecycle::Active)
        || authority.status != AuthorityStatus::Ready
    {
        return None;
    }
    let expected = authority.expected();
    let request = snapshot.nearby.iter().find(|device| {
        device.peer_id == peer_id
            && device
                .link
                .as_ref()
                .is_some_and(|link| matches!(link.state, NearbyState::Confirm {}))
    });
    if let Some(device) = request {
        return Some((
            device.name.clone(),
            request_card(device, catalog, withheld, expected),
        ));
    }
    let peer = snapshot.peers.iter().find(|peer| peer.peer_id == peer_id)?;
    Some((
        peer.name.clone(),
        linked_card(snapshot, peer, catalog, expected),
    ))
}

fn request_card(
    device: &qol_peers::admin::NearbyDevice,
    catalog: Option<&[CatalogOperation]>,
    withheld: &[PluginId],
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let grants = grants(catalog, withheld);
    let mut rows = vec![Row::new(
        "Link",
        format!("Only if {} shows {}", device.name, link_code(device)),
        Some(Action::Send(Request::Nearby {
            request: NearbyRequest::Confirm {
                expected,
                peer_id: device.peer_id,
                grants,
            },
        })),
    )
    .chip("link")];
    rows.extend(new_link_permissions(catalog, withheld));
    rows.push(
        Row::new(
            "Decline",
            "Neither device links",
            Some(decline(device.peer_id, expected)),
        )
        .chip("decline"),
    );
    rows
}

fn grants(catalog: Option<&[CatalogOperation]>, withheld: &[PluginId]) -> Vec<OperationKey> {
    catalog
        .into_iter()
        .flatten()
        .filter(|operation| !withheld.contains(&operation.plugin_id))
        .map(|operation| operation.key.clone())
        .collect()
}

fn nearby_rows(
    snapshot: &Snapshot,
    catalog: Option<&[CatalogOperation]>,
    withheld: &[PluginId],
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = vec![Row::header("nearby", "devices on this network")];
    for device in &snapshot.nearby {
        let link = Action::Send(Request::Nearby {
            request: NearbyRequest::Link {
                expected,
                peer_id: device.peer_id,
                grants: grants(catalog, withheld),
            },
        });
        match device.link.as_ref().map(|link| link.state) {
            None => rows.push(
                Row::new(&device.name, "On this network, not linked", Some(link)).chip("link"),
            ),
            Some(NearbyState::Connecting {}) => rows.push(
                Row::new(&device.name, "Asking it for a code", None)
                    .value("connecting", SettingsValueTone::Muted),
            ),
            Some(NearbyState::Failed { error }) => {
                rows.push(Row::new(&device.name, failure_detail(error), Some(link)).chip("retry"))
            }
            Some(NearbyState::Confirm {} | NearbyState::WaitingForPeer {}) => {}
        }
    }
    if snapshot.nearby.is_empty() {
        rows.push(Row::new(
            "Looking for devices",
            "Open Linked devices on the other device. Both need to be on this network.",
            None,
        ));
    }
    if rows.len() == 1 {
        rows.clear();
    }
    rows
}

fn plugins(catalog: Option<&[CatalogOperation]>) -> Vec<&CatalogOperation> {
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
}

fn new_link_permissions(catalog: Option<&[CatalogOperation]>, withheld: &[PluginId]) -> Vec<Row> {
    plugins(catalog)
        .into_iter()
        .map(|operation| {
            Row::new(
                format!("Let it use {}", plugin_label(&operation.plugin_id)),
                operation.description.clone(),
                Some(Action::Toggle(operation.plugin_id.clone())),
            )
            .toggle(!withheld.contains(&operation.plugin_id))
        })
        .collect()
}

fn linked_rows(snapshot: &Snapshot, expected: ExpectedAuthority) -> Vec<Row> {
    let mut rows = vec![Row::header(
        "linked devices",
        "devices that can reach this one",
    )];
    if snapshot.peers.is_empty() {
        rows.push(Row::new(
            "No linked devices yet",
            "Link one from the nearby list",
            None,
        ));
    }
    for peer in &snapshot.peers {
        rows.push(
            Row::new(
                peer.name.clone(),
                connection_detail(snapshot, peer.peer_id),
                Some(Action::Open(peer.peer_id)),
            )
            .verb("open")
            .remove(
                "unlink",
                Action::Send(Request::Revoke {
                    expected,
                    peer_id: peer.peer_id,
                }),
            ),
        );
    }
    rows
}

fn connection_detail(snapshot: &Snapshot, peer_id: PeerId) -> &'static str {
    if snapshot
        .sessions
        .iter()
        .any(|session| session.peer_id == peer_id)
    {
        "Authenticated connection at last refresh"
    } else {
        "Not connected at last refresh"
    }
}

fn linked_card(
    snapshot: &Snapshot,
    peer: &qol_peers::admin::PeerSummary,
    catalog: Option<&[CatalogOperation]>,
    expected: ExpectedAuthority,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let granted = snapshot
        .grants
        .iter()
        .find(|(id, _)| *id == peer.peer_id)
        .map(|(_, grants)| grants.as_slice())
        .unwrap_or_default();
    let set = |grants: Vec<OperationKey>| {
        Some(Action::Send(Request::SetGrants {
            expected,
            peer_id: peer.peer_id,
            grants,
        }))
    };
    for plugin in plugins(catalog) {
        let owned: Vec<_> = catalog
            .into_iter()
            .flatten()
            .filter(|operation| operation.plugin_id == plugin.plugin_id)
            .map(|operation| &operation.key)
            .collect();
        let allowed = owned.iter().all(|key| granted.contains(key));
        let updated = if allowed {
            granted
                .iter()
                .filter(|key| !owned.contains(key))
                .cloned()
                .collect()
        } else {
            let mut updated = granted.to_vec();
            updated.extend(
                owned
                    .into_iter()
                    .filter(|key| !granted.contains(key))
                    .cloned(),
            );
            updated
        };
        rows.push(
            Row::new(
                format!("Can use {}", plugin_label(&plugin.plugin_id)),
                plugin.description.clone(),
                set(updated),
            )
            .toggle(allowed),
        );
    }
    for grant in granted {
        if catalog
            .into_iter()
            .flatten()
            .any(|operation| operation.key == *grant)
        {
            continue;
        }
        rows.push(
            Row::new(
                format!("Can use {}", grant_label(grant)),
                "Its plugin is not installed here",
                set(granted
                    .iter()
                    .filter(|key| *key != grant)
                    .cloned()
                    .collect()),
            )
            .chip("remove"),
        );
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
    rows.push(
        Row::new(
            "Unlink",
            "Both devices forget the link",
            Some(Action::Send(Request::Revoke {
                expected,
                peer_id: peer.peer_id,
            })),
        )
        .chip("unlink"),
    );
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
        "when the devices cannot see each other",
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
                    "Paste it on the other device within two minutes",
                    Some(Action::Copy),
                )
                .chip("copy"),
            );
            rows.push(
                enroll(
                    "Cancel invitation",
                    "The invitation stops working",
                    EnrollmentRequest::CancelInvitation {
                        expected,
                        invitation: *invitation,
                    },
                )
                .chip("cancel"),
            );
        }
        _ => rows.push(
            enroll(
                "Create invitation",
                "Make a one-time invitation to paste on the other device",
                EnrollmentRequest::CreateInvitation {
                    expected,
                    addresses: Vec::new(),
                },
            )
            .chip("create"),
        ),
    }
    rows.push(
        Row::new(
            "Paste invitation",
            "Link using an invitation copied on the other device",
            Some(Action::Paste),
        )
        .chip("paste"),
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
            )
            .chip("approve"),
        );
        rows.push(
            enroll(
                "Reject request",
                &detail,
                EnrollmentRequest::Reject {
                    expected,
                    key: pending.key,
                },
            )
            .chip("reject"),
        );
    }
    for item in &snapshot.outbound {
        if !matches!(item.state, OutboundEnrollmentState::Pending {})
            || snapshot
                .nearby
                .iter()
                .any(|device| device.peer_id == item.key.peer && device.link.is_some())
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
        "Waiting on the other device \u{b7} {}",
        attempt
            .map(crate::features::linked_devices::settings::attempt_label)
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
            )
            .chip("recover"),
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
        )
        .chip("abandon"),
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
                "pointz phones that reach this device",
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
        "pointz phones that reach this device",
    ));
    if status
        .migration
        .is_some_and(|migration| migration.phones_must_pair_again())
    {
        rows.push(
            Row::new(
                "Some phones must pair again",
                "Not every PointZ pairing could move into linked devices",
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
            )
            .chip("remove"),
        );
    }
    rows
}

fn failure_detail(error: EnrollmentFailure) -> &'static str {
    match error {
        EnrollmentFailure::Transport | EnrollmentFailure::Unavailable => {
            "Could not reach it. Check that Linked devices is open there, then retry."
        }
        EnrollmentFailure::Rejected(EnrollmentRejection::Expired) => {
            "The code ran out before both devices chose Link"
        }
        EnrollmentFailure::Rejected(
            EnrollmentRejection::Cancelled | EnrollmentRejection::InvalidInvitation,
        ) => "It declined, or its request ended",
        EnrollmentFailure::Rejected(EnrollmentRejection::Revoked) => {
            "This device revoked it before, so it cannot link again"
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

fn grant_label(grant: &OperationKey) -> String {
    format!("{} {}", grant.kind.as_str(), grant.name)
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
        AuthorityLifetime::Session => "Links end when qol-tray stops on this device",
        AuthorityLifetime::Persistent => "Links stay on this device",
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

fn send(label: &str, detail: &str, request: Request) -> Row {
    Row::new(label, detail, Some(Action::Send(request)))
}

fn enroll(label: &str, detail: &str, request: EnrollmentRequest) -> Row {
    send(label, detail, Request::Enrollment { request })
}
