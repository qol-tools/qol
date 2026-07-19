import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionLabel, actionShowsActivity, selectedActionName } from './action-state.js';

test('runtime-active actions switch between explicit start and stop contracts', () => {
    const field = {
        action: 'start_search',
        active_action: 'stop_search',
        label: 'Start search',
        active_label: 'Stop search',
    };
    assert.equal(selectedActionName(field, false), 'start_search');
    assert.equal(selectedActionName(field, true), 'stop_search');
    assert.equal(actionLabel(field, false, false, false), 'Start search');
    assert.equal(actionLabel(field, false, true, false), 'Stop search');
    assert.equal(actionLabel(field, true, true, false), 'Working...');
});

test('ordinary and pairing actions preserve their existing labels', () => {
    assert.equal(actionLabel({ action: 'reload', label: 'Reload' }, false, false, false), 'Reload');
    assert.equal(actionLabel({ action: 'pair', label: 'Pair' }, false, false, true), 'Stop Pairing');
    assert.equal(selectedActionName({ action: 'reload' }, true), 'reload');
});

test('persistent toggle actions do not present their active state as ongoing work', () => {
    assert.equal(actionShowsActivity({ variant: 'toggle' }, true), false);
    assert.equal(actionShowsActivity({ variant: 'primary' }, true), true);
    assert.equal(actionShowsActivity({ variant: 'toggle' }, false), false);
});
