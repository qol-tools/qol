import test from 'node:test';
import assert from 'node:assert/strict';
import { declaredFieldsToSchema, groupFields } from './qol-config.js';

test('groupFields partitions directional, scalar, and boolean fields', () => {
    const schema = [
        ['from_mods', 'mods'],
        ['to_key', 'string'],
        ['label', 'string'],
        ['global', 'boolean'],
    ];

    assert.deepEqual(groupFields(schema), {
        fromFields: [['from_mods', 'mods']],
        toFields: [['to_key', 'string']],
        rest: [['label', 'string']],
        booleans: [['global', 'boolean']],
    });
});

test('declaredFieldsToSchema maps contract kinds and modifier arrays', () => {
    const cases = [
        [
            { name: 'string', count: 'number', enabled: 'boolean' },
            [['name', 'string'], ['count', 'number'], ['enabled', 'boolean']],
        ],
        [
            { from_mods: 'string_array', keys: 'string_array' },
            [['from_mods', 'mods'], ['keys', 'string-array']],
        ],
        [{ custom: 'unknown' }, [['custom', 'string']]],
    ];

    for (const [fields, expected] of cases) {
        assert.deepEqual(declaredFieldsToSchema(fields), expected, JSON.stringify(fields));
    }
});
