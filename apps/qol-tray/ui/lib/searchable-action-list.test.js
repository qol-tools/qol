import { test } from 'node:test';
import assert from 'node:assert/strict';
import { filterSearchableItems, firstSearchableItemId } from './searchable-action-list.js';

const items = [
    {
        id: 'speaker',
        label: 'Luna 2',
        description: 'Available · -42 dBm',
        actionLabel: 'Pair',
        actions: [{ id: 'trust', label: 'Trust' }, { id: 'remove', label: 'Remove' }],
        keywords: ['AA:BB:CC:DD:EE:01'],
    },
    {
        id: 'headphones',
        label: 'Headphones',
        description: 'Connected',
        actionLabel: 'Disconnect',
        keywords: ['AA:BB:CC:DD:EE:02'],
    },
];

test('searchable action list filters every user-visible result attribute', () => {
    const cases = [
        ['empty query', '', ['speaker', 'headphones']],
        ['name', 'luna', ['speaker']],
        ['description', '-42', ['speaker']],
        ['action', 'disconnect', ['headphones']],
        ['secondary action', 'remove', ['speaker']],
        ['keyword', 'EE:01', ['speaker']],
        ['no match', 'keyboard', []],
    ];
    for (const [label, query, expected] of cases) {
        assert.deepEqual(
            filterSearchableItems(items, query).map(item => item.id),
            expected,
            `case: ${label}`,
        );
    }
});

test('firstSearchableItemId safely resolves navigation entry', () => {
    assert.equal(firstSearchableItemId(items), 'speaker');
    assert.equal(firstSearchableItemId([]), null);
});
