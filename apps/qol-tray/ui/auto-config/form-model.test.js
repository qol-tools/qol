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
