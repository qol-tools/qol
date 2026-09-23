import { matchesQuery } from '../utils/collections.js';

export function filterSearchableItems(items, query) {
    const normalized = String(query ?? '').trim();
    if (!normalized) return items;
    return items.filter(item => matchesQuery([
        item.label,
        item.description,
        item.actionLabel,
        ...(item.actions || []).map(action => action.label),
        ...(item.keywords || []),
    ], normalized));
}

export function firstSearchableItemId(items) {
    return items[0]?.id ?? null;
}
