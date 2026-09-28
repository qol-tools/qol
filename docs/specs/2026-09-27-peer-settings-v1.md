# Linked devices settings v1

## Ownership and scope

Expose the implemented peer authority in core settings. Native and web controls use the same headless Request/Response contract through the existing authenticated local API and runtime socket. Neither UI constructs PeerAuthority, reads its files, maintains trust or grants, performs discovery, or dispatches plugin operations independently.

This is an intermediate implementation slice. PointZ migration, Bluetooth handoff, final guest/device verification and the coherent delivery remain required. No placeholder or simulated control may imply working earbuds.

## User flow

Core settings gains a Linked devices entry, using the existing retained settings window and shared controls. The web dashboard gains its corresponding page and supplies the existing native-failure fallback.

Opening either view only reads state. An inactive service offers explicit setup with a computer name and a clear choice between this session and persistent links on this computer. Unsupported persistence and unavailable/standby/stopping states are visible. Changing lifetime never silently destroys or replaces the active authority.

Active state shows this computer's name and lifetime, linked devices, fresh connection status and pending pairing requests. Rename, stop and revoke use the displayed authority identity, activation and revision. Destructive transitions are confirmed through the existing UI pattern.

Pairing has two paths: create/copy an invitation, or paste an invitation from another computer. Invitations are displayed as an opaque link code, not an editable JSON configuration. Core validates the code and owns prepare/redeem/approve/reject/cancel/abandon/recover. Show the authenticated peer name and identity when confirming a request. Pairing grants no operations automatically. No IP entry is required.

Read-only polling can refresh progress. A mutation is submitted once. An uncertain reply remains uncertain until canonical state is read; it never triggers an automatic replacement mutation or silently overwrites a stale authority stamp. Outbound attempts retain their canonical transaction identity. Recovery uses that identity and original invitation endpoints, never a new prepare to disguise uncertainty. Invitations live only in transient UI memory and explicitly requested clipboard copies; no local/session storage, URLs, profile/config writes, traces or error echoes contain them. A visible explicit copy action may use the clipboard.

Peer details show current grants and available peer-exposed local operations from the existing canonical catalog. Preserve every existing grant, including currently unavailable operations; unknown labels do not delete permissions. Changes use SetGrants and its expected authority stamp. Catalog, grant and availability facts are derived and never persisted by the presentation layer.

Connection labels use authenticated sessions. Discovery advertisements and stored trust do not mean connected. On failed or inconsistent refresh, stop presenting old connection state as current. Pagination and asynchronous replies are bound to authority identity/activation/revision; network session pagination also uses its network revision. Late replies cannot overwrite a newer view or a mutation result.

## Local API and automatic invitation addresses

Add a thin authenticated settings route under the existing API middleware. Forward canonical peer administration through PlatformStateClient to the existing host, using this runtime generation's socket. Return the canonical response and preserve typed transport uncertainty. Apply the existing message bound before deserialization, no-store response headers, and the established token/origin policy; no public enrollment/admin HTTP listener or new network channel.

A separate read-only operation projection derives only explicit peer exposure from the existing operation_catalog owner. Capture declarations under PluginManager's guard and resolve files outside it, following the implemented capture/resolve API. The host remains responsible for validating any selected grants.

An empty CreateInvitation.addresses means select this computer's usable IPv4 addresses in core. Existing explicit address callers retain their behavior. Resolve addresses outside host/manager guards before final expected-authority validation and invitation creation. Network port and endpoint validation stay with NetworkControl. No background address registry or UI discovery loop is needed.

The address leaf belongs to libs/peers service networking. Reuse the already locked if-addrs 0.15.0 implementation as an optional service dependency if no existing shared owner supplies this operation. Read current interfaces, filter unusable/non-IPv4/loopback entries for production, deduplicate and deterministically bound to eight; an empty/error result refuses invitation creation. Use a small injectable leaf for deterministic tests. Loopback is fixture-only under the existing explicit opt-in. No subprocess, firewall edits, listening socket, credential creation or discovery is performed merely to render settings.

## Presentation and verification

Reuse the settings component register, theme tokens, keyboard navigation, existing Preact/htm and world-page patterns. Keep long codes and errors bounded. Native requests run off the UI thread and GPUI's bounded executor workers; keep one operation in flight, stop polling when parked, and discard stale replies. Web work is abortable/read-only on unmount; never retry an ambiguous mutation.

Meaningful tests cover the actual route to an isolated attached authority, authentication/body boundary, canonical errors and uncertain delivery, coherent pagination/stale responses, preservation of hidden/unavailable grants, and automatic-address refusal/selection. Reuse the real two-host enrollment fixture to exercise automatic invitation setup with injected addresses and unchanged approval/recovery semantics. Pure presenter/keyboard tests need no real compositor. Native rendering, focus and end-user pairing are verified centrally in a disposable guest; no live host changes or installed binary replacement.
