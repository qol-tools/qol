export const enrollment = (action, fields = {}) => ({ operation: 'enrollment', request: { action, ...fields } });
export const pointz = (action, fields = {}) => ({ operation: 'pointz', request: { action, ...fields } });
export const phoneRemoval = (snapshot, phone) => structuredClone(pointz('remove',
    { expected: stamp(snapshot.status.authority), device_id: phone.device_id }));
export const phonesMustPairAgain = migration => Boolean(migration
    && (migration.seed_replaced || migration.devices_unreadable || migration.dropped > 0));
export const stamp = authority => ({ authority_id: authority.peer_id, activation_id: authority.activation_id, revision: authority.revision });
export const sameStamp = (a, b) => a?.authority_id === b?.authority_id && a?.activation_id === b?.activation_id && a?.revision === b?.revision;
export const operationId = key => JSON.stringify([key.identity?.scope, key.identity?.value, key.kind, key.name]);
export const withoutGrant = (grants, removed) => grants.filter(key => operationId(key) !== operationId(removed));
export const viewKey = authority => authority ? `${authority.peer_id}:${authority.activation_id}:${authority.revision}` : 'inactive';

export function permissionChanges(snapshot, peer, catalog) {
    const grants = snapshot.grants[peer.peer_id];
    if (!grants) return [];
    const request = updated => structuredClone({ operation: 'set_grants',
        expected: stamp(snapshot.status.authority), peer_id: peer.peer_id, grants: updated });
    const remove = grants.map(key => ({ action: 'remove', key, label: operationId(key),
        request: request(withoutGrant(grants, key)) }));
    const add = (catalog || []).filter(operation => !grants.some(key => operationId(key) === operationId(operation.key)))
        .map(operation => ({ action: 'add', key: operation.key,
            label: `${operation.plugin_id} · ${operation.description} (${operation.key.kind} ${operation.key.name})`,
            request: request([...grants, operation.key]) }));
    return [...remove, ...add];
}

function inconsistent() { throw { kind: 'inconsistent' }; }

async function pages(request, makeRequest, expected, result, networkRevision) {
    let cursor = { ...expected, offset: 0, ...(networkRevision === undefined ? {} : { network_revision: networkRevision }) };
    const items = [];
    let total;
    for (;;) {
        const response = await request(makeRequest(cursor));
        const page = response.page;
        if (response.result !== result || !page || !sameStamp(page.cursor, expected)
            || page.cursor.offset !== cursor.offset || page.cursor.network_revision !== networkRevision
            || (total !== undefined && page.total !== total)) inconsistent();
        total = page.total;
        items.push(...page.items);
        if (!page.next) {
            if (items.length !== total) inconsistent();
            return items;
        }
        if (!sameStamp(page.next, expected) || page.next.network_revision !== networkRevision
            || page.next.offset !== items.length || page.next.offset <= cursor.offset || page.next.offset >= total) inconsistent();
        cursor = page.next;
    }
}

export async function loadSnapshot(request) {
    const first = await request({ operation: 'status' });
    if (first.result !== 'status') inconsistent();
    const snapshot = { status: first.status, peers: [], sessions: [], pending: [], outbound: [], grants: {}, attempts: {}, pointz: null, phones: [] };
    if (!first.status.authority || first.status.lifecycle.state !== 'active'
        || first.status.authority.status !== 'ready') return snapshot;
    const expected = stamp(first.status.authority);
    snapshot.peers = await pages(request, cursor => ({ operation: 'peers', cursor }), expected, 'peers');
    for (const peer of snapshot.peers) {
        snapshot.grants[peer.peer_id] = await pages(async query => {
            const response = await request(query);
            if (response.peer_id !== peer.peer_id) inconsistent();
            return response;
        }, cursor => ({ operation: 'grants', peer_id: peer.peer_id, cursor }), expected, 'grants');
    }
    const pending = await request(enrollment('pending'));
    if (pending.result !== 'pending_enrollments' || !sameStamp(pending.authority, expected)) inconsistent();
    snapshot.pending = pending.items;
    snapshot.outbound = await pages(request, cursor => enrollment('outbound', { cursor }), expected, 'outbound_enrollments');
    for (const item of snapshot.outbound) {
        const transaction = item.key.transaction;
        const attempt = await request(enrollment('attempt', { expected, transaction }));
        if (attempt.result !== 'enrollment_attempt' || !sameStamp(attempt.authority, expected) || attempt.transaction !== transaction) inconsistent();
        snapshot.attempts[transaction] = attempt.state;
    }
    const network = await request({ operation: 'network' });
    if (network.result !== 'network' || !sameStamp(network.network.authority, expected)) inconsistent();
    snapshot.network = network.network;
    snapshot.sessions = await pages(request, cursor => ({ operation: 'sessions', cursor }), expected, 'sessions', network.network.network_revision);
    const phones = await request(pointz('status'));
    if (phones.result !== 'pointz_status' || !sameStamp(phones.status.authority, expected)) inconsistent();
    snapshot.pointz = phones.status;
    if (phones.status.migration) {
        snapshot.phones = await pages(request, cursor => pointz('devices', { cursor }), expected, 'pointz_devices');
    }
    const endNetwork = await request({ operation: 'network' });
    const last = await request({ operation: 'status' });
    if (last.result !== 'status' || JSON.stringify(last.status) !== JSON.stringify(first.status)
        || endNetwork.result !== 'network' || !sameStamp(endNetwork.network.authority, expected)
        || endNetwork.network.network_revision !== network.network.network_revision) inconsistent();
    return snapshot;
}

export function failureMessage(error) {
    if (error?.kind === 'outcome_unknown') return 'Result unknown. Refresh canonical state before another change. Recover the original transaction; do not prepare a replacement.';
    if (error?.kind === 'invalid_request') return 'Invalid or oversized request. Check the device name or invitation code; nothing was submitted to core.';
    if (error?.kind === 'access_denied') return 'Local API access was refused. Reopen settings from QoL to authenticate.';
    if (error?.kind === 'authority') {
        if (error.error?.code === 'authority' && error.error.error?.code === 'unsupported_platform') return 'Persistent links are unavailable on this platform. Choose this session instead.';
        const code = String(error.error?.error?.code || error.error?.code || 'unavailable').slice(0, 64);
        return `Core refused this change (${code}). Refresh before deciding what to do next.`;
    }
    return 'Current state is unavailable. Refresh to read core again; previous connection labels are no longer current.';
}
