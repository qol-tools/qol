import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

const hooksSource = `
export function useState(initial) {
    const harness = globalThis.__peerHookHarness;
    harness.state = initial;
    return [initial, value => { harness.state = value; }];
}
export function useEffect(effect) { globalThis.__peerHookHarness.effect = effect; }
`;
const apiSource = `
export function peerRequest(query, signal) { return globalThis.__peerHookHarness.request(query, signal); }
export async function invitationInfo() { throw new Error('Unexpected inspection'); }
export async function peerCatalog() { return []; }
`;
const loaderSource = `
const hooks = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(hooksSource))};
const api = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(apiSource))};
export function resolve(specifier, context, nextResolve) {
    if (context.parentURL?.includes('/views/linked-devices/use-peers.js')) {
        if (specifier === 'preact/hooks') return { url: hooks, shortCircuit: true, format: 'module' };
        if (specifier === '../../api/peers.js') return { url: api, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;
register('data:text/javascript,' + encodeURIComponent(loaderSource), pathToFileURL('./'));
const { usePeers } = await import('./use-peers.js');

test('hook remount and automatic entry polling cannot clear dispatched mutation uncertainty', async t => {
    const inactive = { result: 'status', status: { authority: null, lifecycle: { state: 'inactive' } } };
    const calls = [];
    let finish;
    let poll;
    const delayed = new Promise(resolve => { finish = resolve; });
    const harness = {
        request: async query => {
            calls.push(query.operation);
            if (query.operation === 'status') return inactive;
            return delayed;
        },
    };
    globalThis.__peerHookHarness = harness;
    t.mock.method(globalThis, 'setInterval', callback => { poll = callback; return 1; });
    t.mock.method(globalThis, 'clearInterval', () => {});
    const previousDocument = globalThis.document;
    globalThis.document = { hidden: false };
    t.after(() => {
        if (previousDocument === undefined) delete globalThis.document;
        if (previousDocument !== undefined) globalThis.document = previousDocument;
        delete globalThis.__peerHookHarness;
    });
    const first = usePeers(true);
    const unmount = harness.effect();
    await new Promise(resolve => setImmediate(resolve));
    const pending = first.mutate({ operation: 'start_session', name: 'once' });
    unmount();
    const second = usePeers(true);
    const closeSecond = harness.effect();
    assert.equal(harness.state.recoveryRequired, true);
    assert.equal(harness.state.busy, true);
    assert.deepEqual(calls, ['status', 'start_session']);
    finish({ result: 'changed' });
    await pending;
    poll();
    await second.mutate({ operation: 'start_session', name: 'blocked' });
    assert.deepEqual(calls, ['status', 'start_session']);
    assert.equal(harness.state.snapshot, null);
    assert.equal(harness.state.recoveryRequired, true);
    closeSecond();
    const third = usePeers(true);
    const closeThird = harness.effect();
    assert.deepEqual(calls, ['status', 'start_session']);
    await third.refresh();
    assert.equal(harness.state.recoveryRequired, false);
    await third.mutate({ operation: 'start_session', name: 'intentional' });
    assert.deepEqual(calls, ['status', 'start_session', 'status', 'start_session', 'status']);
    closeThird();
});
