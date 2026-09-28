import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSnapshot, withoutGrant, permissionChanges, stamp, failureMessage } from './model.js';

const authority = { peer_id: 'local', activation_id: 'activation', revision: '9007199254740993', status: 'ready' };
const expected = stamp(authority);
const cursor = { ...expected, offset: 0 };
const status = { authority, lifecycle: { state: 'active' } };

function replies(overrides = {}) {
    const page = { cursor, total: 0, items: [], next: null };
    const byOperation = {
        status: { result: 'status', status },
        peers: { result: 'peers', page },
        pending: { result: 'pending_enrollments', authority: expected, items: [] },
        outbound: { result: 'outbound_enrollments', page },
        network: { result: 'network', network: { authority: expected, network_revision: '4' } },
        sessions: { result: 'sessions', page: { ...page, cursor: { ...cursor, network_revision: '4' } } },
        ...overrides,
    };
    return async request => byOperation[request.request?.action || request.operation];
}

test('coherent reads keep decimal revisions as opaque strings', async () => {
    const snapshot = await loadSnapshot(replies());
    assert.equal(snapshot.status.authority.revision, '9007199254740993');
});

test('inconsistent authority and network pages cannot become current connection state', async () => {
    for (const changed of [{ revision: '1' }, { activation_id: 'new' }, { authority_id: 'other' }, { network_revision: '5' }]) {
        await assert.rejects(loadSnapshot(replies({ sessions: {
            result: 'sessions', page: { cursor: { ...cursor, network_revision: '4', ...changed }, total: 0, items: [], next: null },
        } })), error => error.kind === 'inconsistent');
    }
});

test('pagination rejects a backwards or retargeted continuation', async () => {
    for (const next of [cursor, { ...cursor, offset: 1, activation_id: 'other' }]) {
        await assert.rejects(loadSnapshot(replies({ peers: {
            result: 'peers', page: { cursor, total: 2, items: [{ peer_id: 'peer' }], next },
        } })), error => error.kind === 'inconsistent');
    }
});

test('removing one grant preserves every unavailable grant and never mutates the snapshot', () => {
    const grants = [
        { identity: { scope: 'stable', value: 'missing-plugin' }, kind: 'action', name: 'hidden' },
        { identity: { scope: 'stable', value: 'installed' }, kind: 'action', name: 'remove' },
    ];
    assert.deepEqual(withoutGrant(grants, grants[1]), [grants[0]]);
    assert.equal(grants.length, 2);
});

test('permission presenter freezes add and remove requests without losing unavailable grants', () => {
    const key = name => ({ identity: { scope: 'stable', value: 'fixture' }, kind: 'action', name });
    const grants = [key('hidden'), key('granted')];
    const peer = { peer_id: 'selected' };
    const snapshot = { status: { authority: { ...authority } }, grants: { selected: grants } };
    const added = key('new');
    const catalog = [
        { key: { name: 'granted', kind: 'action', identity: { value: 'fixture', scope: 'stable' } }, plugin_id: 'fixture', description: 'Granted' },
        { key: added, plugin_id: 'fixture', description: 'New' },
    ];
    const changes = permissionChanges(snapshot, peer, catalog);
    assert.deepEqual(changes.map(item => item.action), ['remove', 'remove', 'add']);
    assert.deepEqual(changes.map(item => item.request), [
        { operation: 'set_grants', expected, peer_id: 'selected', grants: [grants[1]] },
        { operation: 'set_grants', expected, peer_id: 'selected', grants: [grants[0]] },
        { operation: 'set_grants', expected, peer_id: 'selected', grants: [...grants, added] },
    ]);
    snapshot.status.authority.revision = 'changed';
    peer.peer_id = 'other';
    added.name = 'changed';
    assert.equal(changes[2].request.expected.revision, expected.revision);
    assert.equal(changes[2].request.peer_id, 'selected');
    assert.equal(changes[2].request.grants[2].name, 'new');
    assert.deepEqual(snapshot.grants.selected, [key('hidden'), key('granted')]);
    for (const unavailable of [null, []]) {
        assert.deepEqual(permissionChanges(snapshot, { peer_id: 'selected' }, unavailable).map(item => item.action), ['remove', 'remove']);
    }
    assert.deepEqual(permissionChanges(snapshot, { peer_id: 'missing' }, catalog), []);
});

test('multi-page peer, grant and session reads retain every item', async () => {
    const peers = Array.from({ length: 19 }, (_, index) => ({ peer_id: `peer-${index}` }));
    const grants = Array.from({ length: 21 }, (_, index) => ({ identity: { scope: 'stable', value: 'unavailable' }, kind: 'action', name: `grant-${index}` }));
    const sessions = peers.map(peer => ({ ...peer, generation: { local: 'a', remote: 'b' } }));
    const fallback = replies();
    const paginate = (cursor, items) => {
        const end = Math.min(cursor.offset + 16, items.length);
        return { cursor, total: items.length, items: items.slice(cursor.offset, end), next: end < items.length ? { ...cursor, offset: end } : null };
    };
    const snapshot = await loadSnapshot(async query => {
        if (query.operation === 'peers') return { result: 'peers', page: paginate(query.cursor, peers) };
        if (query.operation === 'grants') return { result: 'grants', peer_id: query.peer_id, page: paginate(query.cursor, grants) };
        if (query.operation === 'sessions') return { result: 'sessions', page: paginate(query.cursor, sessions) };
        return fallback(query);
    });
    assert.deepEqual(snapshot.peers, peers);
    assert.deepEqual(snapshot.sessions, sessions);
    for (const peer of peers) assert.deepEqual(snapshot.grants[peer.peer_id], grants);
});

test('final authority and network rereads reject changes after otherwise coherent pages', async () => {
    for (const changed of ['authority', 'network', 'grant-peer']) {
        const fallback = replies();
        let statuses = 0;
        let networks = 0;
        await assert.rejects(loadSnapshot(async query => {
            if (query.operation === 'status' && ++statuses === 2 && changed === 'authority') {
                return { result: 'status', status: { ...status, authority: { ...authority, revision: '4' } } };
            }
            if (query.operation === 'network' && ++networks === 2 && changed === 'network') {
                return { result: 'network', network: { authority: expected, network_revision: '5' } };
            }
            if (changed === 'grant-peer' && query.operation === 'peers') {
                return { result: 'peers', page: { cursor, total: 1, items: [{ peer_id: 'selected' }], next: null } };
            }
            if (query.operation === 'grants') {
                return { result: 'grants', peer_id: 'other', page: { cursor, total: 0, items: [], next: null } };
            }
            return fallback(query);
        }), error => error.kind === 'inconsistent', changed);
    }
});

test('stopping and faulted status remains visible without reading invalid peer pages', async () => {
    for (const current of [
        { ...status, lifecycle: { state: 'stopping' } },
        { ...status, authority: { ...authority, status: 'faulted' } },
    ]) {
        const calls = [];
        const snapshot = await loadSnapshot(async query => {
            calls.push(query.operation);
            return { result: 'status', status: current };
        });
        assert.deepEqual(calls, ['status']);
        assert.deepEqual(snapshot.status, current);
        assert.deepEqual(snapshot.sessions, []);
    }
});

test('refusal messages expose unsupported persistence and bound diagnostics without invitation echoes', () => {
    assert.match(failureMessage({ kind: 'authority', error: { code: 'authority', error: { code: 'unsupported_platform' } } }), /Choose this session/);
    for (const kind of ['outcome_unknown', 'invalid_request', 'access_denied', 'transport']) {
        const message = failureMessage({ kind, document: 'qol-link:private-fixture' });
        assert.ok(message.length < 256);
        assert.ok(!message.includes('private-fixture'));
    }
});
