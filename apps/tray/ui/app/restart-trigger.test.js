import { test } from 'node:test';
import assert from 'node:assert/strict';
import { postRestartTrigger } from './restart-trigger.js';

function withFetch(impl, run) {
    const original = globalThis.fetch;
    globalThis.fetch = impl;
    return Promise.resolve(run()).finally(() => { globalThis.fetch = original; });
}

test('a severed connection is treated as restart-in-progress, not a failure', async () => {
    let httpErrors = 0;
    await withFetch(
        async () => { throw new TypeError('NetworkError when attempting to fetch resource'); },
        () => postRestartTrigger('/api/x', { method: 'POST' }, () => { httpErrors++; }),
    );
    assert.equal(httpErrors, 0);
});

test('a delivered non-2xx response is reported to the caller', async () => {
    const seen = [];
    await withFetch(
        async () => ({ ok: false, status: 409 }),
        () => postRestartTrigger('/api/x', { method: 'POST' }, res => seen.push(res.status)),
    );
    assert.deepEqual(seen, [409]);
});

test('a 2xx response lets the flow continue silently (driven by SSE)', async () => {
    let httpErrors = 0;
    await withFetch(
        async () => ({ ok: true, status: 202 }),
        () => postRestartTrigger('/api/x', { method: 'POST' }, () => { httpErrors++; }),
    );
    assert.equal(httpErrors, 0);
});

test('restart trigger suppresses only the initiating request toast', async () => {
    const seen = [];
    await withFetch(
        async (_url, opts) => { seen.push(opts); return { ok: true, status: 202 }; },
        () => postRestartTrigger('/api/x', { method: 'POST' }, () => {}),
    );
    assert.equal(seen[0].qolSuppressErrorToast, true);
});
