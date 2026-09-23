export function clampIndex(index, count) {
    return Math.min(index, Math.max(0, count - 1));
}

export function sortByName(items) {
    return [...items].sort((a, b) => {
        const left = String(a?.name ?? '');
        const right = String(b?.name ?? '');
        return left.localeCompare(right);
    });
}

export function matchesQuery(fields, query) {
    if (!query) return true;
    const q = query.toLowerCase();
    return fields.some(f => String(f ?? '').toLowerCase().includes(q));
}
