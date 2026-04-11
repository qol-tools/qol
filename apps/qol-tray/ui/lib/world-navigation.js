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

    function gotoAnchor(anchor, { respectKnob = true } = {}) {
        void anchor;
        void respectKnob;
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
