import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

const hooksStubSource = `
export function useState(initial) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }
export function useCallback(fn) { return fn; }
`;

const loaderSource = `
const STUB_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(hooksStubSource))};
export function resolve(specifier, context, nextResolve) {
    if (specifier === 'preact/hooks') {
        return { url: STUB_URL, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;

register('data:text/javascript,' + encodeURIComponent(loaderSource), pathToFileURL('./'));

const { actionErrorMessage, useDispatchAction } = await import(`./useDispatchAction.js?case=${Date.now()}`);

function withFetch(handler, run) {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = handler;
    return Promise.resolve(run()).finally(() => {
        if (originalFetch === undefined) delete globalThis.fetch;
        else globalThis.fetch = originalFetch;
    });
}

test('action error message extracts structured response messages', () => {
    assert.equal(
        actionErrorMessage(409, '{"success":false,"message":"No coordinator detected"}'),
        'No coordinator detected',
    );
});

test('action error message falls back to text or HTTP status', () => {
    assert.equal(actionErrorMessage(500, 'plain failure'), 'plain failure');
    assert.equal(actionErrorMessage(404, ''), 'HTTP 404');
});

test('dispatch suppresses global fetch error toast for inline action errors', async () => {
    const seen = [];
    await withFetch(
        async (_url, options) => {
            seen.push(options);
            return new Response('{"success":false,"message":"No coordinator"}', { status: 409 });
        },
        async () => {
            const action = useDispatchAction('plugin-lights', 'reload');
            await assert.rejects(() => action.dispatch(), /No coordinator/);
        },
    );

    assert.equal(seen.length, 1);
    assert.equal(seen[0].qolSuppressErrorToast, true);
});

test('dispatch carries row input to an action selected at activation time', async () => {
    const seen = [];
    await withFetch(
        async (url, options) => {
            seen.push({ url, options });
            return new Response('{"success":true}', { status: 200 });
        },
        async () => {
            const action = useDispatchAction('plugin-bluetooth', null);
            await action.dispatch({ address: 'AA:BB' }, 'pair_device');
        },
    );

    assert.equal(seen[0].url, '/api/plugins/plugin-bluetooth/actions/pair_device');
    assert.equal(seen[0].options.body, '{"address":"AA:BB"}');
});
