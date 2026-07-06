import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isActionRuntimeGated } from './field-rules.js';

test('action runtime gate disables normal actions when runtime is unhealthy', () => {
    assert.equal(isActionRuntimeGated({ action: 'pair' }, true), true);
    assert.equal(isActionRuntimeGated({ action: 'toggle_main' }, true), true);
});

test('action runtime gate keeps reload and ghost recovery actions available', () => {
    assert.equal(isActionRuntimeGated({ action: 'reload' }, true), false);
    assert.equal(isActionRuntimeGated({ action: 'scan', variant: 'ghost' }, true), false);
});

test('action runtime gate does nothing while runtime is healthy', () => {
    assert.equal(isActionRuntimeGated({ action: 'pair' }, false), false);
});
