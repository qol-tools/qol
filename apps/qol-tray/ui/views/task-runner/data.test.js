import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createEditModalState } from './data.js';

test('new task-runner action uses host-served timeout default', () => {
    const modal = createEditModalState({}, null, { actionTimeout: 42 });
    assert.equal(modal.timeout, 42);
    assert.equal(modal.defaultTimeout, 42);
});

test('existing task-runner action keeps its configured timeout', () => {
    const modal = createEditModalState(
        { build: { name: 'Build', command: 'make', timeout: 7 } },
        'build',
        { actionTimeout: 42 },
    );
    assert.equal(modal.timeout, 7);
    assert.equal(modal.defaultTimeout, 42);
});
