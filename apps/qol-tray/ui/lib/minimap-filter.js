export function visibleMinimapEntries({ allEntries, confinedPages, diveParent }) {
    if (confinedPages && confinedPages.length > 0) {
        const allowed = new Set(confinedPages);
        return allEntries.filter(e => allowed.has(e.id));
    }
    if (diveParent) return allEntries.filter(e => e.parent === diveParent);
    return allEntries;
}

// Slice a sorted entry list down to a window of (radius*2 + 1) entries
// centred on the active page. When the active page is near either edge,
// the window slides so the result still contains 2*radius + 1 entries
// (clamped to the input length). A radius of 0 or non-positive disables
// slicing — callers pass that to mean "show everything". When `activeId`
// is missing or not found, we fall back to the leading window.
export function sliceMinimapWindow({ entries, activeId, radius }) {
    if (!Array.isArray(entries) || entries.length === 0) return entries;
    if (!Number.isFinite(radius) || radius <= 0) return entries;
    const r = Math.floor(radius);
    const span = r * 2 + 1;
    if (span >= entries.length) return entries;
    const idx = activeId == null ? -1 : entries.findIndex(e => e.id === activeId);
    if (idx < 0) return entries.slice(0, span);
    let start = idx - r;
    let end = idx + r + 1;
    if (start < 0) { end -= start; start = 0; }
    if (end > entries.length) { start -= (end - entries.length); end = entries.length; }
    if (start < 0) start = 0;
    return entries.slice(start, end);
}
