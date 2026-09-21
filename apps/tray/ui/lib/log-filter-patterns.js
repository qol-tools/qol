export function patternsFromInput(raw) {
    if (!raw) return [];
    return raw.split(',').map(v => v.trim()).filter(Boolean);
}

export function patternsToInput(patterns) {
    return Array.isArray(patterns) ? patterns.join(', ') : '';
}
