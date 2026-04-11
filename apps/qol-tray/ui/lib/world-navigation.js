import { createDebug } from './debug.js';

const log = createDebug('qol:nav-state');

export function createNavigation({ registry, camera, getSettings, domHelpers }) {
    let currentAnchor = { pageId: null };
    const focusRegistry = {};
    const diveStack = [];

    function getCurrentAnchor() {
        return currentAnchor;
    }

    function setCurrentAnchor(anchor) {
        currentAnchor = { pageId: anchor.pageId };
    }

    function setFocus(pageId, selector) {
        if (!pageId || !selector) return;
        focusRegistry[pageId] = selector;
    }

    function getFocus(pageId) {
        return focusRegistry[pageId] || null;
    }

    function resolvePageEntry(pageId) {
        const entry = registry.getEntry(pageId);
        if (entry) return entry;
        log('gotoAnchor: unknown pageId, falling back', pageId);
        const fallback = registry.getEntriesForLayer(0)[0];
        return fallback || null;
    }

    function resolveCenter(entry, pageId) {
        const selector = focusRegistry[pageId];
        if (selector) {
            const resolved = domHelpers.resolveSelector(selector);
            if (resolved) return { x: resolved.x, y: resolved.y };
        }
        return { x: entry.x + entry.width / 2, y: entry.y + entry.height / 2 };
    }

    function gotoAnchor(anchor, { respectKnob = true } = {}) {
        if (!anchor || !anchor.pageId) {
            log('gotoAnchor: skipped (no anchor)');
            return;
        }
        if (respectKnob && getSettings().anchorToPages === false) {
            log('gotoAnchor: skipped (knob off)', anchor.pageId);
            return;
        }
        const entry = resolvePageEntry(anchor.pageId);
        if (!entry) {
            log('gotoAnchor: skipped (no fallback entry)', anchor.pageId);
            return;
        }
        const center = resolveCenter(entry, anchor.pageId);
        const { w, h } = domHelpers.getViewportSize();
        const z = camera.zoom || 1;
        const targetX = center.x - w / (2 * z);
        const targetY = center.y - h / (2 * z);
        if (entry.layer !== camera.layer) {
            camera.setLayer(entry.layer);
        }
        camera.panSmooth(targetX, targetY, 400);
    }

    function dive(targetPageId) {
        if (!targetPageId) {
            log('dive: skipped (no targetPageId)');
            return;
        }
        diveStack.push({ anchor: currentAnchor, zoom: camera.zoom });
        currentAnchor = { pageId: targetPageId };
        gotoAnchor(currentAnchor, { respectKnob: false });
    }

    function ascend() {
        const prev = diveStack.pop();
        if (!prev) {
            log('ascend: skipped (empty stack)');
            return false;
        }
        currentAnchor = prev.anchor;
        camera.zoomTo(prev.zoom);
        gotoAnchor(prev.anchor, { respectKnob: false });
        return true;
    }

    function stackDepth() {
        return diveStack.length;
    }

    return {
        getCurrentAnchor,
        setCurrentAnchor,
        setFocus,
        getFocus,
        dive,
        ascend,
        gotoAnchor,
        stackDepth,
    };
}
