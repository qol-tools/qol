export function visibleMinimapEntries({ allEntries, confinedPages, diveParent }) {
    if (confinedPages && confinedPages.length > 0) {
        const allowed = new Set(confinedPages);
        return allEntries.filter(e => allowed.has(e.id));
    }
    if (diveParent) return allEntries.filter(e => e.parent === diveParent);
    return allEntries;
}
