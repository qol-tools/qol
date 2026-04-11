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

    function dive(targetPageId) {
        throw new Error('dive: not implemented');
    }

    function ascend() {
        throw new Error('ascend: not implemented');
    }

    function gotoAnchor(anchor, opts) {
        throw new Error('gotoAnchor: not implemented');
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
