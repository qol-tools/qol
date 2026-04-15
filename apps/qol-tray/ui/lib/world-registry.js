const PAGE_WIDTH = 1280;
const PAGE_HEIGHT = 900;
const PAGE_STRIDE = 10000;

export function contains(rect, entry) {
    if (!rect) return true;
    if (entry.layer !== rect.layer) return false;
    return entry.x >= rect.x
        && entry.y >= rect.y
        && entry.x + entry.width <= rect.x + rect.width
        && entry.y + entry.height <= rect.y + rect.height;
}

export function createWorldRegistry(viewOrder, manifest = {}) {
    const entries = new Map();
    const diveTargets = new Map();

    function addEntry(entry) {
        entries.set(entry.id, { ...entry });
    }

    function addDiveTarget(target) {
        const traits = target.traits || { confined: {} };
        diveTargets.set(target.sourceSelector, { ...target, traits });
    }

    function getDiveTargets() {
        return Array.from(diveTargets.values());
    }

    function getDiveTargetForSource(selector) {
        return diveTargets.get(selector) || null;
    }

    for (let i = 0; i < viewOrder.length; i++) {
        const id = viewOrder[i];
        entries.set(id, {
            id, x: i * PAGE_STRIDE, y: 0,
            width: PAGE_WIDTH, height: PAGE_HEIGHT,
            layer: 0, parent: null,
        });
    }

    for (const [parentId, subs] of Object.entries(manifest)) {
        const parent = entries.get(parentId);
        if (!parent) continue;
        for (let i = 0; i < subs.length; i++) {
            const subId = `${parentId}-${subs[i]}`;
            entries.set(subId, {
                id: subId,
                x: parent.x,
                y: parent.y,
                width: PAGE_WIDTH, height: PAGE_HEIGHT,
                layer: -1, parent: parentId,
            });
        }
    }

    function getEntry(id) {
        return entries.get(id) || null;
    }

    function getAllEntries() {
        return Array.from(entries.values());
    }

    function getEntriesForLayer(n) {
        return getAllEntries().filter(e => e.layer === n);
    }

    function getSubPages(parentId) {
        return getAllEntries().filter(e => e.parent === parentId);
    }

    function diveTarget(id) {
        return entries.get(id) || null;
    }

    function worldBounds(layerFilter) {
        const pool = layerFilter !== undefined
            ? getAllEntries().filter(e => e.layer === layerFilter)
            : getAllEntries();
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const e of pool) {
            minX = Math.min(minX, e.x);
            minY = Math.min(minY, e.y);
            maxX = Math.max(maxX, e.x + e.width);
            maxY = Math.max(maxY, e.y + e.height);
        }
        if (minX === Infinity) return { x: 0, y: 0, width: 0, height: 0 };
        const pad = 100;
        return { x: minX - pad, y: minY - pad, width: maxX - minX + pad * 2, height: maxY - minY + pad * 2 };
    }

    function activeViewId(cameraX, cameraY, viewportW, viewportH, zoom) {
        const z = zoom || 1;
        const cx = cameraX + viewportW / (2 * z);
        const cy = cameraY + viewportH / (2 * z);
        let closest = null;
        let closestDist = Infinity;
        for (const e of entries.values()) {
            if (e.layer !== 0) continue;
            const vx = e.x + e.width / 2;
            const vy = e.y + e.height / 2;
            const d = Math.hypot(cx - vx, cy - vy);
            if (d < closestDist) { closest = e.id; closestDist = d; }
        }
        return closest;
    }

    function placeNew(id, width, height) {
        const w = width || PAGE_WIDTH;
        const h = height || PAGE_HEIGHT;
        let maxRight = 0;
        for (const e of entries.values()) {
            if (e.layer !== 0) continue;
            maxRight = Math.max(maxRight, e.x + PAGE_STRIDE);
        }
        const entry = { id, x: maxRight, y: 0, width: w, height: h, layer: 0, parent: null };
        entries.set(id, entry);
        return entry;
    }

    function cameraTargetForView(id, viewportW, viewportH, zoom) {
        const e = entries.get(id);
        if (!e) return null;
        const z = zoom || 1;
        return {
            x: e.x + e.width / 2 - viewportW / (2 * z),
            y: e.y + e.height / 2 - viewportH / (2 * z),
        };
    }

    return {
        getEntry,
        getAllEntries,
        getEntriesForLayer,
        getSubPages,
        diveTarget,
        worldBounds,
        activeViewId,
        placeNew,
        cameraTargetForView,
        addEntry,
        addDiveTarget,
        getDiveTargets,
        getDiveTargetForSource,
    };
}
