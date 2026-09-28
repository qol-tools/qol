import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createPeerController } from './controller.js';

const inactive = { result: 'status', status: { authority: null, lifecycle: { state: 'inactive' } } };
const deferred = () => {
    let resolve;
    let reject;
    const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
};

function fixture(request, inspectInvitation = async () => ({ invitation: 'invitation', peer: 'peer', endpoints: ['127.0.0.1:1'] })) {
    const updates = [];
    const controller = createPeerController({ request, inspectInvitation, readCatalog: async () => [], publish: value => updates.push(value) });
    controller.setActive(true);
    return { controller, updates, state: () => updates.at(-1) };
}

test('an uncertain mutation is sent once and only a successful explicit read permits another change', async () => {
    const calls = [];
    const view = fixture(async query => {
        calls.push(query);
        if (query.operation === 'status') return inactive;
        throw { kind: 'outcome_unknown' };
    });
    const change = { operation: 'start_session', name: 'fixture' };
    await view.controller.mutate(change);
    await view.controller.mutate(change);
    await view.controller.poll();
    assert.equal(calls.length, 1);
    assert.equal(view.state().snapshot, null);
    assert.match(view.state().message, /Result unknown/);
    await view.controller.refresh();
    assert.deepEqual(view.state().snapshot.status, inactive.status);
    await view.controller.mutate(change);
    assert.deepEqual(calls.map(query => query.operation), ['start_session', 'status', 'start_session']);
});

test('parked reads retain one in-flight operation and discard late replies before another change', async () => {
    const delayed = deferred();
    let firstSignal;
    let count = 0;
    const view = fixture(async (query, signal) => {
        count += 1;
        if (count === 1) { firstSignal = signal; return delayed.promise; }
        if (query.operation === 'enrollment') return { result: 'invitation', document: 'fresh-code' };
        return inactive;
    });
    const old = view.controller.refresh();
    view.controller.setActive(false);
    assert.equal(firstSignal.aborted, true);
    view.controller.setActive(true);
    await view.controller.mutate({ operation: 'enrollment', request: { action: 'create_invitation' } });
    assert.equal(count, 1);
    assert.equal(view.state().busy, true);
    delayed.resolve({ result: 'status', status: { authority: null, lifecycle: { state: 'standby' } } });
    await old;
    assert.equal(view.state().snapshot, null);
    assert.equal(view.state().busy, false);
    await view.controller.mutate({ operation: 'enrollment', request: { action: 'create_invitation' } });
    assert.equal(view.state().invitation.document, 'fresh-code');
    assert.equal(view.state().snapshot.status.lifecycle.state, 'inactive');
});

test('one in-flight operation freezes its peer, authority stamp and original recovery transaction', async () => {
    const delayed = deferred();
    const calls = [];
    const view = fixture(async query => {
        calls.push(query);
        if (query.operation === 'status') return inactive;
        return delayed.promise;
    });
    const expected = { authority_id: 'local', activation_id: 'activation', revision: '3' };
    const query = { operation: 'enrollment', request: {
        action: 'recover', expected, transaction: 'original', endpoints: ['127.0.0.1:1'],
    } };
    const pending = view.controller.mutate(query);
    expected.revision = '4';
    query.request.transaction = 'replacement';
    await view.controller.refresh();
    await view.controller.mutate({ operation: 'revoke', peer_id: 'different' });
    assert.equal(calls.length, 1);
    assert.equal(calls[0].request.transaction, 'original');
    assert.equal(calls[0].request.expected.revision, '3');
    delayed.resolve({ result: 'changed' });
    await pending;
    assert.deepEqual(calls.map(item => item.operation), ['enrollment', 'status']);
});

test('failed refresh clears old connection state and keeps mutations blocked after uncertainty', async () => {
    let fail = false;
    const calls = [];
    const view = fixture(async query => {
        calls.push(query.operation);
        if (fail) throw { kind: query.operation === 'status' ? 'transport' : 'outcome_unknown' };
        return inactive;
    });
    await view.controller.refresh();
    fail = true;
    await view.controller.mutate({ operation: 'start_session', name: 'fixture' });
    await view.controller.refresh();
    await view.controller.mutate({ operation: 'start_session', name: 'replacement' });
    await view.controller.poll();
    assert.deepEqual(calls, ['status', 'start_session', 'status']);
    assert.equal(view.state().snapshot, null);
    assert.equal(view.state().busy, false);
});

test('leaving the page clears invitation secrets and discards delayed inspection', async () => {
    const delayed = deferred();
    const view = fixture(async () => inactive, () => delayed.promise);
    const pending = view.controller.inspect('private-code');
    view.controller.setActive(false);
    delayed.resolve({ invitation: 'invitation', peer: 'peer', endpoints: [] });
    await pending;
    assert.equal(view.state().source, null);
    assert.equal(view.state().invitation, null);
    assert.equal(view.state().snapshot, null);
});

test('dispatched mutations remain uncertain through departure, re-entry and stale completion until explicit recovery', async () => {
    for (const outcome of ['applied', 'unknown']) {
        const delayed = deferred();
        const calls = [];
        let dispatchedSignal;
        let failRead = true;
        const view = fixture(async (query, signal) => {
            calls.push(structuredClone(query));
            if (calls.length === 1) { dispatchedSignal = signal; return delayed.promise; }
            if (query.operation === 'status') {
                if (failRead) throw { kind: 'transport' };
                return inactive;
            }
            return { result: 'changed' };
        });
        const change = { operation: 'enrollment', request: { action: 'recover',
            expected: { authority_id: 'local', activation_id: 'activation', revision: '3' },
            transaction: 'original', endpoints: ['127.0.0.1:1'] } };
        const frozen = structuredClone(change);
        const pending = view.controller.mutate(change);
        change.request.transaction = 'replacement';
        change.request.expected.revision = '4';
        view.controller.setActive(false);
        view.controller.setActive(true);
        assert.equal(dispatchedSignal.aborted, true);
        assert.equal(view.state().recoveryRequired, true);
        await view.controller.poll();
        await view.controller.refresh();
        await view.controller.mutate(change);
        assert.deepEqual(calls, [frozen]);
        if (outcome === 'applied') delayed.resolve({ result: 'invitation', document: 'stale-secret' });
        if (outcome === 'unknown') delayed.reject({ kind: 'outcome_unknown' });
        await pending;
        assert.equal(view.state().busy, false);
        assert.equal(view.state().snapshot, null);
        assert.equal(view.state().invitation, null);
        assert.equal(view.state().recoveryRequired, true);
        assert.match(view.state().message, /Result unknown/);
        await view.controller.poll();
        await view.controller.mutate(change);
        assert.deepEqual(calls, [frozen]);
        await view.controller.refresh();
        assert.equal(view.state().recoveryRequired, true);
        failRead = false;
        view.controller.setActive(false);
        view.controller.setActive(true);
        await view.controller.poll();
        await view.controller.mutate(change);
        assert.equal(calls.length, 2);
        await view.controller.refresh();
        assert.equal(view.state().recoveryRequired, false);
        assert.deepEqual(view.state().snapshot.status, inactive.status);
        await view.controller.mutate(change);
        assert.deepEqual(calls, [frozen, { operation: 'status' }, { operation: 'status' }, change, { operation: 'status' }]);
    }
});

test('a failed catalog read stays unavailable while a successful empty catalog is distinguishable', async () => {
    let state;
    let fail = true;
    const controller = createPeerController({ request: async () => inactive,
        readCatalog: async () => { if (fail) throw { kind: 'transport' }; return []; },
        publish: value => { state = value; } });
    controller.setActive(true);
    await controller.refresh();
    assert.deepEqual(state.snapshot.status, inactive.status);
    assert.equal(state.catalog, null);
    fail = false;
    await controller.refresh();
    assert.deepEqual(state.catalog, []);
});
