import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionLabel, actionRuntimeState, actionShowsActivity, selectedActionName } from './action-state.js';

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

test('a failed state query shows the toggle unavailable until the query recovers', () => {
    const field = {
        action: 'enable_adapter',
        active_action: 'disable_adapter',
        active_query: 'adapter_status',
        active_value_from: 'powered',
        variant: 'toggle',
        label: 'Bluetooth',
        active_label: 'Bluetooth',
    };
    const steps = [
        [{ data: { powered: true }, error: null }, { active: true, unavailable: false }, 'Bluetooth', 'disable_adapter'],
        [{ data: { powered: true }, error: 'Bluetooth adapter is unavailable' }, { active: false, unavailable: true }, 'Unavailable', 'enable_adapter'],
        [{ data: { powered: false }, error: null }, { active: false, unavailable: false }, 'Bluetooth', 'enable_adapter'],
    ];
    for (const [queryState, expected, label, action] of steps) {
        const state = actionRuntimeState(field, queryState);
        assert.deepEqual(state, expected);
        assert.equal(actionLabel(field, false, state.active, false, state.unavailable), label);
        assert.equal(selectedActionName(field, state.active), action);
    }
});

test('fields without a state query never report unavailable', () => {
    assert.deepEqual(actionRuntimeState({ action: 'reload' }, { data: null, error: 'boom' }), { active: false, unavailable: false });
});
