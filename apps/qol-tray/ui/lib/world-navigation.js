import { createDebug } from './debug.js';
import { getWorldSettings } from './world-settings.js';

const log = createDebug('qol:nav-state');

export function createNavigation({ registry, camera, getSettings, domHelpers }) {
    const STORAGE_KEY = 'qoltray.navigation';
    const LEGACY_KEY = 'qoltray.camera';

    function loadFromStorage() {
        if (typeof localStorage === 'undefined') return null;
        try {
            localStorage.removeItem(LEGACY_KEY);
        } catch {}
        try {
            const raw = localStorage.getItem(STORAGE_KEY);
            if (!raw) return null;
            const parsed = JSON.parse(raw);
            if (parsed && typeof parsed === 'object') return parsed;
        } catch {}
        return null;
    }

    function saveToStorage() {
        if (typeof localStorage === 'undefined') return;
        try {
            const snapshot = {
                currentAnchor: { pageId: currentAnchor.pageId },
                zoom: camera.zoom,
                focusRegistry: { ...focusRegistry },
            };
            localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
        } catch {}
    }

    let saveTimer = 0;
    function scheduleSave() {
        if (typeof clearTimeout === 'function') clearTimeout(saveTimer);
        saveTimer = setTimeout(saveToStorage, 300);
    }

    const persisted = loadFromStorage();
    let currentAnchor = persisted?.currentAnchor?.pageId
        ? { pageId: persisted.currentAnchor.pageId }
        : { pageId: null };
    const focusRegistry = (persisted?.focusRegistry && typeof persisted.focusRegistry === 'object')
        ? { ...persisted.focusRegistry }
        : {};
    const diveStack = [];
    let currentConfinement = null;
    if (persisted?.zoom && typeof camera.zoomTo === 'function') {
        camera.zoomTo(persisted.zoom);
    }

    function getCurrentConfinement() {
        return currentConfinement;
    }

    function setBounds(rect) {
        if (typeof camera.setBounds === 'function') camera.setBounds(rect);
    }

    function getCurrentAnchor() {
        return currentAnchor;
    }

    function setCurrentAnchor(anchor) {
        currentAnchor = { pageId: anchor.pageId };
        scheduleSave();
    }

    function setFocus(pageId, selector) {
        if (!pageId || !selector) return;
        focusRegistry[pageId] = selector;
        scheduleSave();
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

    function resolveCenter(entry) {
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
        const center = resolveCenter(entry);
        const { w, h } = domHelpers.getViewportSize();
        const z = camera.zoom || 1;
        const targetX = center.x - w / (2 * z);
        const targetY = center.y - h / (2 * z);
        if (entry.layer !== camera.layer) camera.setLayer(entry.layer);
        const pageId = anchor.pageId;
        camera.panSmooth(targetX, targetY, 400, () => {
            if (typeof document === 'undefined') return;
            const sel = focusRegistry[pageId];
            if (!sel) return;
            const el = document.querySelector(sel);
            if (el && typeof el.focus === 'function') el.focus({ preventScroll: true });
        });
    }

    function dive(targetPageId) {
        if (!targetPageId) {
            log('dive: skipped (no targetPageId)');
            return;
        }
        diveStack.push({
            anchor: currentAnchor,
            zoom: camera.zoom,
            layer: camera.layer,
            confinement: currentConfinement,
        });
        currentAnchor = { pageId: targetPageId };
        gotoAnchor(currentAnchor, { respectKnob: false });
        scheduleSave();
    }

    function diveInto(sourceSelector) {
        const target = registry.getDiveTargetForSource?.(sourceSelector);
        if (!target) {
            log('diveInto: no target for', sourceSelector);
            return;
        }
        diveStack.push({
            anchor: currentAnchor,
            zoom: camera.zoom,
            layer: camera.layer,
            confinement: currentConfinement,
        });
        currentConfinement = target.claim;
        setBounds(currentConfinement);
        const firstPageId = target.pages[0];
        if (firstPageId) {
            currentAnchor = { pageId: firstPageId };
            gotoAnchor(currentAnchor, { respectKnob: false });
        }
        scheduleSave();
    }

    function ascend() {
        const prev = diveStack.pop();
        if (!prev) {
            log('ascend: skipped (empty stack)');
            return false;
        }
        currentAnchor = prev.anchor;
        currentConfinement = prev.confinement ?? null;
        setBounds(currentConfinement);
        camera.zoomTo(prev.zoom);
        gotoAnchor(prev.anchor, { respectKnob: false });
        scheduleSave();
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
        diveInto,
        ascend,
        gotoAnchor,
        stackDepth,
        getCurrentConfinement,
    };
}

export function selectorFor(el) {
    if (!el) return null;
    if (el.id) return `#${CSS.escape(el.id)}`;
    const viewId = el.closest('[data-view-id]')?.dataset?.viewId;
    const index = el.getAttribute('data-index');
    if (viewId && index != null) {
        return `[data-view-id="${CSS.escape(viewId)}"] [data-selected-surface][data-index="${index}"]`;
    }
    return null;
}

export function animateTransition(vp, animatingRef, outClass, applyLayer, onDone) {
    const { transitionStyle, transitionSpeed } = getWorldSettings();
    const minimap = document.querySelector('.world-minimap-container');
    if (transitionStyle === 'instant') {
        applyLayer();
        if (onDone) onDone();
        return;
    }
    animatingRef.current = true;
    const totalBudget = transitionSpeed * 3;
    const failsafe = setTimeout(() => {
        clearAnimClass(vp, 'dive-out');
        clearAnimClass(vp, 'ascend-out');
        clearAnimClass(vp, 'fade-out');
        clearAnimClass(vp, 'layer-in');
        clearAnimClass(minimap, 'dive-out');
        clearAnimClass(minimap, 'ascend-out');
        clearAnimClass(minimap, 'fade-out');
        clearAnimClass(minimap, 'layer-in');
        animatingRef.current = false;
    }, totalBudget);
    const outAnim = transitionStyle === 'fade' ? 'fade-out' : outClass;
    const dur = `${transitionSpeed}ms`;
    const durIn = `${Math.round(transitionSpeed * 0.6)}ms`;
    applyAnimClass(vp, outAnim, dur);
    applyAnimClass(minimap, outAnim, dur);
    vp.addEventListener('animationend', function onEnd(e) {
        if (e.target !== vp) return;
        vp.removeEventListener('animationend', onEnd);
        clearAnimClass(vp, outAnim);
        clearAnimClass(minimap, outAnim);
        applyLayer();
        applyAnimClass(vp, 'layer-in', durIn);
        applyAnimClass(minimap, 'layer-in', durIn);
        vp.addEventListener('animationend', function onIn(e) {
            if (e.target !== vp) return;
            vp.removeEventListener('animationend', onIn);
            clearAnimClass(vp, 'layer-in');
            clearAnimClass(minimap, 'layer-in');
            clearTimeout(failsafe);
            animatingRef.current = false;
        });
        if (onDone) onDone();
    });
}

function applyAnimClass(el, cls, dur) {
    if (!el) return;
    el.style.animationDuration = dur;
    el.classList.add(cls);
}

function clearAnimClass(el, cls) {
    if (!el) return;
    el.classList.remove(cls);
    el.style.animationDuration = '';
}
