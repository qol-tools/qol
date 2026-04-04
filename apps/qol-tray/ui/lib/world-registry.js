const DEFAULT_VIEW_WIDTH = 1000;
const DEFAULT_VIEW_HEIGHT = 800;
const GAP = 200;

export function createWorldRegistry(viewOrder) {
    const entries = new Map();
    let nextX = 0;

    for (const id of viewOrder) {
        entries.set(id, { id, x: nextX, y: 0, width: DEFAULT_VIEW_WIDTH, height: DEFAULT_VIEW_HEIGHT });
        nextX += DEFAULT_VIEW_WIDTH + GAP;
    }

    function getEntry(id) {
        return entries.get(id) || null;
    }

    function getAllEntries() {
        return Array.from(entries.values());
    }

    function worldBounds() {
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const e of entries.values()) {
            minX = Math.min(minX, e.x);
            minY = Math.min(minY, e.y);
            maxX = Math.max(maxX, e.x + e.width);
            maxY = Math.max(maxY, e.y + e.height);
        }
        if (minX === Infinity) return { x: 0, y: 0, width: 0, height: 0 };
        const pad = 100;
        return { x: minX - pad, y: minY - pad, width: maxX - minX + pad * 2, height: maxY - minY + pad * 2 };
    }

    function activeViewId(cameraX, cameraY, viewportW, viewportH) {
        const cx = cameraX + viewportW / 2;
        const cy = cameraY + viewportH / 2;
        let closest = null;
        let closestDist = Infinity;
        for (const e of entries.values()) {
            const vx = e.x + e.width / 2;
            const vy = e.y + e.height / 2;
            const d = Math.hypot(cx - vx, cy - vy);
            if (d < closestDist) { closest = e.id; closestDist = d; }
        }
        return closest;
    }

    function placeNew(id, width, height) {
        const w = width || DEFAULT_VIEW_WIDTH;
        const h = height || DEFAULT_VIEW_HEIGHT;
        let bestX = 0;
        for (const e of entries.values()) {
            bestX = Math.max(bestX, e.x + e.width + GAP);
        }
        const entry = { id, x: bestX, y: 0, width: w, height: h };
        entries.set(id, entry);
        return entry;
    }

    function cameraTargetForView(id, viewportW, viewportH) {
        const e = entries.get(id);
        if (!e) return null;
        return {
            x: e.x + e.width / 2 - viewportW / 2,
            y: e.y + e.height / 2 - viewportH / 2,
        };
    }

    return {
        getEntry,
        getAllEntries,
        worldBounds,
        activeViewId,
        placeNew,
        cameraTargetForView,
    };
}
