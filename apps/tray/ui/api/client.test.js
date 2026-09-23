import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resetAuthLostState } from '../lib/http-auth.js';

let nextResponse = () => new Response(null, { status: 401 });
const events = [];
const storage = { removed: [], removeItem(key) { this.removed.push(key); } };
const win = {
    __QOL_HTTP_TOKEN__: 'tok',
    fetch: async () => nextResponse(),
    dispatchEvent(ev) { events.push(ev); },
    CustomEvent: class CustomEvent { constructor(type) { this.type = type; } },
};
globalThis.window = win;
globalThis.location = { origin: 'http://127.0.0.1:42700' };
globalThis.sessionStorage = storage;
globalThis.document = { cookie: 'qol_token=tok; SameSite=Strict; Path=/' };

await import('../api/client.js');

test('401 on a dashboard api call declares auth lost once and silences toasts', async () => {
    resetAuthLostState();
    events.length = 0;
    storage.removed = [];
    win.__QOL_HTTP_TOKEN__ = 'tok';
    globalThis.document.cookie = 'qol_token=tok; SameSite=Strict; Path=/';

    await win.fetch('/api/events', { method: 'POST' });
    assert.deepEqual(storage.removed, ['qol:http-token'], 'session token cleared');
    assert.equal(globalThis.document.cookie, 'qol_token=; Max-Age=0; Path=/; SameSite=Strict');
    assert.equal(win.__QOL_HTTP_TOKEN__, null);
    assert.deepEqual(
        events.map(e => e.type),
        ['qol:http-auth-lost'],
        'auth-lost fires once; no error toast for the 401'
    );

    events.length = 0;
    await win.fetch('/api/events', { method: 'POST' });
    assert.deepEqual(events.map(e => e.type), [], 'later 401s stay silent');
});

test('non-401 errors still toast and a cross-origin 401 is not auth loss', async () => {
    resetAuthLostState();
    events.length = 0;
    nextResponse = () => new Response('boom', { status: 500 });
    await win.fetch('/api/events', { method: 'POST' });
    assert.deepEqual(events.map(e => e.type), ['app-toast'], '500 still toasts');

    events.length = 0;
    nextResponse = () => new Response(null, { status: 401 });
    await win.fetch('http://127.0.0.1:42999/api/x', { method: 'POST' });
    assert.deepEqual(
        events.map(e => e.type),
        ['app-toast'],
        'cross-origin 401 is a foreign server, not dashboard auth loss'
    );
});
