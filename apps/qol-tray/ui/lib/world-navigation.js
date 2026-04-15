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
                layer: camera.layer,
                camX: camera.x,
                camY: camera.y,
                confinement: currentConfinement,
                confinedPages: [...currentConfinedPages],
                traits: { ...currentTraits },
                sourceSelector: currentSourceSelector,
                diveStack: diveStack.map(f => ({ ...f })),
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
    const diveStack = Array.isArray(persisted?.diveStack) ? persisted.diveStack.map(f => ({ ...f })) : [];
    let currentConfinement = persisted?.confinement ?? null;
    let currentConfinedPages = Array.isArray(persisted?.confinedPages) ? [...persisted.confinedPages] : [];
    let currentTraits = (persisted?.traits && typeof persisted.traits === 'object') ? { ...persisted.traits } : {};
    let currentSourceSelector = typeof persisted?.sourceSelector === 'string' ? persisted.sourceSelector : null;
    if (persisted?.zoom && typeof camera.zoomTo === 'function') {
        camera.zoomTo(persisted.zoom);
    }
    if (Number.isInteger(persisted?.layer) && typeof camera.setLayer === 'function') {
        camera.setLayer(persisted.layer);
    }
    if (typeof persisted?.camX === 'number' && typeof persisted?.camY === 'number' && typeof camera.panTo === 'function') {
        camera.panTo(persisted.camX, persisted.camY);
    }

    const anchorListeners = new Set();

    function notifyAnchorChange() {
        for (const fn of anchorListeners) fn(currentAnchor);
    }

    function subscribeAnchor(fn) {
        anchorListeners.add(fn);
        return () => anchorListeners.delete(fn);
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
        notifyAnchorChange();
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

    function resolvePageEntry(pageId, { allowFallback = true } = {}) {
        const entry = registry.getEntry(pageId);
        if (entry) return entry;
        if (!allowFallback) return null;
        log('gotoAnchor: unknown pageId, falling back', pageId);
        const fallback = registry.getEntriesForLayer(0)[0];
        return fallback || null;
    }

    function resolveCenter(entry) {
        return { x: entry.x + entry.width / 2, y: entry.y + entry.height / 2 };
    }

    function gotoAnchor(anchor, { respectKnob = true, instant = false } = {}) {
        if (!anchor || !anchor.pageId) {
            log('gotoAnchor: skipped (no anchor)');
            return;
        }
        if (respectKnob && getSettings().anchorToPages === false) {
            log('gotoAnchor: skipped (knob off)', anchor.pageId);
            return;
        }
        const entry = resolvePageEntry(anchor.pageId, { allowFallback: !instant });
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
        const focusAfterPan = () => {
            if (typeof document === 'undefined') return;
            const sel = focusRegistry[pageId];
            if (sel) {
                const el = document.querySelector(sel);
                if (el && typeof el.focus === 'function') {
                    el.focus({ preventScroll: true });
                    return;
                }
            }
            const slot = document.querySelector(`[data-view-id="${CSS.escape(pageId)}"]`);
            const firstSurface = slot?.querySelector?.('[data-selected-surface]');
            if (firstSurface && typeof firstSurface.focus === 'function') {
                firstSurface.focus({ preventScroll: true });
            }
        };
        if (instant) {
            camera.panTo(targetX, targetY);
            focusAfterPan();
            return;
        }
        const dx = targetX - camera.x;
        const dy = targetY - camera.y;
        const distance = Math.sqrt(dx * dx + dy * dy);
        const FAR_THRESHOLD = 2000;
        const APPROACH_PX = 400;

        if (distance > FAR_THRESHOLD) {
            const ratio = APPROACH_PX / distance;
            camera.panTo(targetX - dx * ratio, targetY - dy * ratio);
            camera.panSmooth(targetX, targetY, 120, focusAfterPan);
            return;
        }
        camera.panSmooth(targetX, targetY, 200, focusAfterPan);
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
        notifyAnchorChange();
        gotoAnchor(currentAnchor, { respectKnob: false });
        scheduleSave();
    }

    function diveInto(sourceSelector) {
        const target = registry.getDiveTargetForSource?.(sourceSelector);
        if (!target) {
            log('diveInto: no target for', sourceSelector);
            return null;
        }
        log('diveInto: saving layer', camera.layer,
            'anchor', currentAnchor?.pageId || 'none',
            '→', target.pages[0] || 'no pages');
        diveStack.push({
            anchor: currentAnchor,
            zoom: camera.zoom,
            layer: camera.layer,
            confinement: currentConfinement,
            confinedPages: currentConfinedPages,
            traits: currentTraits,
            sourceSelector: currentSourceSelector,
        });
        currentConfinement = target.claim;
        currentConfinedPages = target.pages || [];
        currentTraits = target.traits || {};
        currentSourceSelector = sourceSelector;
        setBounds(null);
        const firstPageId = target.pages[0];
        if (firstPageId) {
            currentAnchor = { pageId: firstPageId };
            notifyAnchorChange();
            gotoAnchor(currentAnchor, { respectKnob: false });
        } else if (target.claim.layer !== camera.layer && typeof camera.setLayer === 'function') {
            camera.setLayer(target.claim.layer);
        }
        scheduleSave();
        return target;
    }

    function ascend() {
        const prev = diveStack.pop();
        if (!prev) {
            log('ascend: skipped (empty stack)');
            return false;
        }
        log('ascend: prev.layer', prev.layer,
            'camera.layer', camera.layer,
            'anchor', prev.anchor?.pageId || 'none',
            'stack', diveStack.length);
        currentAnchor = prev.anchor;
        notifyAnchorChange();
        currentConfinement = prev.confinement ?? null;
        currentConfinedPages = prev.confinedPages ?? [];
        currentTraits = prev.traits ?? {};
        currentSourceSelector = prev.sourceSelector ?? null;
        if (typeof prev.zoom === 'number' && typeof camera.zoomTo === 'function') {
            camera.zoomTo(prev.zoom);
        }
        setBounds(null);
        const targetLayer = typeof prev.layer === 'number' ? prev.layer : 0;
        const anchorEntry = prev.anchor?.pageId ? registry.getEntry(prev.anchor.pageId) : null;
        const anchorOnCorrectLayer = anchorEntry && anchorEntry.layer === targetLayer;
        const safeAnchor = anchorOnCorrectLayer
            ? prev.anchor
            : { pageId: registry.getEntriesForLayer(targetLayer)[0]?.id };
        if (typeof camera.setLayer === 'function') {
            camera.setLayer(targetLayer);
        }
        if (safeAnchor?.pageId) {
            gotoAnchor(safeAnchor, { respectKnob: false });
        }
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
        getConfinedPages() { return currentConfinedPages; },
        getCurrentTraits() { return currentTraits; },
        getCurrentSourceSelector() { return currentSourceSelector; },
        refreshCurrentDive() {
            if (!currentSourceSelector) return false;
            const target = registry.getDiveTargetForSource?.(currentSourceSelector);
            if (!target) return false;
            currentConfinement = target.claim;
            currentConfinedPages = target.pages || [];
            currentTraits = target.traits || {};
            const anchorStillValid = currentConfinedPages.includes(currentAnchor?.pageId);
            if (!anchorStillValid && currentConfinedPages.length > 0) {
                currentAnchor = { pageId: currentConfinedPages[0] };
                notifyAnchorChange();
                gotoAnchor(currentAnchor, { respectKnob: false });
            }
            return true;
        },
        subscribeAnchor,
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
