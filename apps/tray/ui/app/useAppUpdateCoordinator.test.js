import { test } from 'node:test';
import assert from 'node:assert/strict';
import { routeSSEReconnect, routeUpdateSSE } from './update-sse-routing.js';

test('prod reconnect reloads after a finished update (status done)', () => {
    const cases = ['downloading', 'done'];
    for (const status of cases) {
        assert.equal(
            routeSSEReconnect(false, status, () => 'dev'),
            'update',
            `status: ${status}`,
        );
    }
});

test('prod reconnect does not reload when no update is in flight', () => {
    const cases = ['idle', 'checking', 'up-to-date', 'available', 'error'];
    for (const status of cases) {
        assert.equal(routeSSEReconnect(false, status, () => 'dev'), null, `status: ${status}`);
    }
});

test('dev reconnect delegates to the dev flow reconnect', () => {
    assert.equal(routeSSEReconnect(true, 'downloading', () => 'recompile'), 'recompile');
    assert.equal(routeSSEReconnect(true, 'done', () => null), null);
});

test('update_complete marks the flow done without re-checking the API', () => {
    const calls = [];
    routeUpdateSSE({ type: 'update_complete' }, state => calls.push(state));
    assert.deepEqual(calls, [{ status: 'done' }]);
});

test('update_progress reports download percent', () => {
    const calls = [];
    routeUpdateSSE({ type: 'update_progress', percent: 42 }, state => calls.push(state));
    assert.deepEqual(calls, [{ status: 'downloading', percent: 42 }]);
});

test('update_failed surfaces an error', () => {
    const calls = [];
    routeUpdateSSE({ type: 'update_failed' }, state => calls.push(state));
    assert.deepEqual(calls, [{ status: 'error' }]);
});
