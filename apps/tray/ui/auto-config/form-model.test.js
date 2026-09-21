import test from 'node:test';
import assert from 'node:assert/strict';
import { configFromForm, ownedConfigKeys } from './form-model.js';

function formWith(fields) {
    return { fields, sections: [] };
}

test('runtime-only fields never become persisted configuration', () => {
    const runtimeKinds = ['action', 'list', 'status', 'qr_code', 'gamepad'];
    const fields = runtimeKinds.map((kind, index) => ({
        id: `runtime_${index}`,
        kind,
        value: `ignored_${kind}`,
    }));
    fields.push({ id: 'stored', kind: 'string', value: 'kept' });
    const form = formWith(fields);

    assert.deepEqual(configFromForm(form), { stored: 'kept' });
    assert.deepEqual([...ownedConfigKeys(form)], ['stored']);
});

test('live number fields never hydrate or own a saved key', () => {
    const form = formWith([
        {
            id: 'volume',
            kind: 'number',
            value: 0,
            config_key: 'output.volume',
            active_query: 'volume',
            active_value_from: 'volume',
            action: 'set_volume',
        },
        { id: 'stored', kind: 'string', value: 'kept' },
    ]);

    assert.deepEqual(configFromForm(form), { stored: 'kept' });
    assert.deepEqual([...ownedConfigKeys(form)], ['stored']);
});

test('live select fields still hydrate and own their saved key', () => {
    const form = formWith([
        {
            id: 'output_device',
            kind: 'select',
            value: 'default',
            config_key: 'output.device',
            active_query: 'output_status',
            active_value_from: 'shown',
        },
    ]);

    assert.deepEqual(configFromForm(form), { output: { device: 'default' } });
    assert.deepEqual([...ownedConfigKeys(form)], ['output']);
});
