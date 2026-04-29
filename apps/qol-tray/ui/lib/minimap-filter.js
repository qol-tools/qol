export function visibleMinimapEntries({ allEntries, confinedPages, diveParent }) {
    if (confinedPages && confinedPages.length > 0) {
        const allowed = new Set(confinedPages);
        return allEntries.filter(e => allowed.has(e.id));
    }
    if (diveParent) return allEntries.filter(e => e.parent === diveParent);
    return allEntries;
}

// Restrict a sorted entry list to those whose world-x bounds intersect a
// world-x range. Used by the minimap so its zoom tracks the viewport's
// zoom: callers compute `worldStart`/`worldEnd` from the camera centre and
// the viewport's world-x range scaled by a user-configurable factor. An
// invalid range (NaN bounds, end <= start) returns the input untouched —
// the caller falls back to showing everything in that case.
export function sliceMinimapRange({ entries, worldStart, worldEnd }) {
    if (!Array.isArray(entries) || entries.length === 0) return entries;
    if (!Number.isFinite(worldStart) || !Number.isFinite(worldEnd)) return entries;
    if (worldEnd <= worldStart) return entries;
    return entries.filter(e => {
        const eEnd = e.x + e.width;
        return eEnd > worldStart && e.x < worldEnd;
    });
}
