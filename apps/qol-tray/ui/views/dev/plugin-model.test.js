import { test } from 'node:test';
import assert from 'node:assert/strict';

import { mergePlugins } from './plugin-model.js';

test('mergePlugins carries version from linked plugins through to the merged entry', () => {
    const cases = [
        {
            name: 'linked only',
            discovered: [],
            linked: [{ id: 'a', name: 'A', source: '/src/a', version: '2.0.0' }],
            expected: '2.0.0'
        },
        {
            name: 'discovered then linked promotes version',
            discovered: [{ id: 'a', name: 'A', path: '/src/a' }],
            linked: [{ id: 'a', name: 'A', source: '/src/a', version: '3.1.0' }],
            expected: '3.1.0'
        },
        {
            name: 'discovered local-only has no version',
            discovered: [{ id: 'b', name: 'B', path: '/src/b' }],
            linked: [],
            expected: ''
        }
    ];
    for (const { name, discovered, linked, expected } of cases) {
        const merged = mergePlugins(discovered, linked);
        const entry = merged.find(p => p.id === (linked[0]?.id ?? discovered[0]?.id));
        assert.equal(entry.version, expected, `case: ${name}`);
    }
});
