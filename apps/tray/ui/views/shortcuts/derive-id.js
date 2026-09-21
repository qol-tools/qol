function slugify(s) {
    return String(s ?? '')
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-+|-+$/g, '')
        .slice(0, 64)
        .replace(/-+$/g, '');
}

export function deriveShortcutId(name, existingIds = [], fallback = '') {
    const base = slugify(name) || slugify(fallback) || 'shortcut';
    const taken = new Set(existingIds);
    if (!taken.has(base)) return base;
    let n = 2;
    while (taken.has(`${base}-${n}`)) n++;
    return `${base}-${n}`;
}
