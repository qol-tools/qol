import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    AUTH_LOST_EVENT,
    clearTokenEvidence,
    declareAuthLost,
    isAuthLost,
    resetAuthLostState,
} from './http-auth.js';

function fakeEnv() {
    const storage = { removeItem: (key) => { storage.removed = [...(storage.removed || []), key]; } };
    const events = [];
    const win = {
        __QOL_HTTP_TOKEN__: 'tok',
        CustomEvent: class CustomEvent {
            constructor(type) { this.type = type; }
        },
        dispatchEvent(event) { events.push(event); },
    };
    const doc = { cookie: 'qol_token=tok; SameSite=Strict; Path=/' };
    return { env: { storage, doc, win }, storage, doc, win, events };
}

test('clearTokenEvidence wipes storage, cookie and window token', () => {
    const { env, storage, doc, win } = fakeEnv();
    clearTokenEvidence(env);
    assert.deepEqual(storage.removed, ['qol:http-token']);
    assert.equal(doc.cookie, 'qol_token=; Max-Age=0; Path=/; SameSite=Strict');
    assert.equal(win.__QOL_HTTP_TOKEN__, null);
});

test('clearTokenEvidence tolerates absent browser pieces', () => {
    assert.doesNotThrow(() => clearTokenEvidence({}));
    assert.doesNotThrow(() => clearTokenEvidence());
});

test('declareAuthLost is single-shot and dispatches exactly one event', () => {
    const { env, events } = fakeEnv();
    assert.equal(declareAuthLost(env), true, 'first declaration wins');
    assert.equal(declareAuthLost(env), false, 'later declarations are ignored');
    assert.equal(events.length, 1, 'event fires once');
    assert.equal(events[0].type, AUTH_LOST_EVENT);
    assert.equal(isAuthLost(), true);
});

test('auth lost state resets for a fresh page load', () => {
    resetAuthLostState();
    assert.equal(isAuthLost(), false);
    const { env } = fakeEnv();
    assert.equal(declareAuthLost(env), true);
    resetAuthLostState();
    assert.equal(declareAuthLost(env), true, 'a reload can declare auth loss again');
    resetAuthLostState();
});

test('declareAuthLost clears token evidence alongside the event', () => {
    const { env, storage, win } = fakeEnv();
    resetAuthLostState();
    declareAuthLost(env);
    assert.deepEqual(storage.removed, ['qol:http-token']);
    assert.equal(win.__QOL_HTTP_TOKEN__, null);
});
