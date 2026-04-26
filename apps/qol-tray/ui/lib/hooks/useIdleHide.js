import { useEffect } from 'preact/hooks';

const DEFAULT_OCCLUDE_SELECTOR = '.world-view-slot';
const OVERLAY_ANCESTOR = '.peripheral-preview, .peripheral-edge-dock';
const HIDE_DELAY_MS = 250;

export function useOverlayHide({
    targetRef,
    camera,
    navigation,
    alwaysVisible = false,
    occludeSelector = DEFAULT_OCCLUDE_SELECTOR,
}) {
    useEffect(
        () => attach(targetRef?.current, camera, navigation, alwaysVisible, occludeSelector),
        [targetRef, camera, navigation, alwaysVisible, occludeSelector],
    );
}

export const useViewportHide = useOverlayHide;

function attach(el, camera, navigation, alwaysVisible, occludeSelector) {
    if (!el) return undefined;
    if (alwaysVisible) {
        el.style.setProperty('--hide', '0');
        el.setAttribute('data-occluded', '0');
        return undefined;
    }
    el.style.setProperty('--hide', '0');
    el.setAttribute('data-occluded', '0');
    const ctx = { el, home: measureHomeRect(el), rafId: 0, hideTimer: 0, occludeSelector };
    const recompute = () => scheduleFrame(ctx);
    const remeasure = () => { ctx.home = measureHomeRect(el); recompute(); };
    recompute();
    return wireListeners(ctx, camera, navigation, recompute, remeasure);
}

function scheduleFrame(ctx) {
    if (ctx.rafId) return;
    ctx.rafId = requestAnimationFrame(() => {
        ctx.rafId = 0;
        evaluate(ctx);
    });
}

function evaluate(ctx) {
    if (!ctx.home) return;
    const overlaps = hasOverlap(ctx.home, ctx.el, ctx.occludeSelector);
    ctx.el.setAttribute('data-occluded', overlaps ? '1' : '0');
    if (overlaps) {
        if (ctx.hideTimer) return;
        ctx.hideTimer = setTimeout(() => {
            ctx.hideTimer = 0;
            ctx.el.style.setProperty('--hide', '1');
        }, HIDE_DELAY_MS);
        return;
    }
    if (ctx.hideTimer) {
        clearTimeout(ctx.hideTimer);
        ctx.hideTimer = 0;
    }
    ctx.el.style.setProperty('--hide', '0');
}

function hasOverlap(home, self, selector) {
    for (const el of document.querySelectorAll(selector)) {
        if (self.contains(el) || el.contains(self)) continue;
        if (el.closest(OVERLAY_ANCESTOR)) continue;
        if (!hasVisibleContent(el)) continue;
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) continue;
        if (rectsOverlap(home, r)) return true;
    }
    return false;
}

function hasVisibleContent(el) {
    if (el.textContent && el.textContent.trim().length > 0) return true;
    if (el.querySelector('img, svg, canvas, video')) return true;
    return false;
}

function rectsOverlap(a, b) {
    return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

function measureHomeRect(el) {
    const hadHide = el.style.getPropertyValue('--hide');
    if (hadHide === '' || parseFloat(hadHide) === 0) {
        return rectOf(el);
    }
    const hadTransition = el.style.getPropertyValue('transition');
    el.style.transition = 'none';
    el.style.setProperty('--hide', '0');
    void el.offsetWidth;
    const rect = rectOf(el);
    el.style.setProperty('--hide', hadHide);
    if (hadTransition) el.style.transition = hadTransition;
    else el.style.removeProperty('transition');
    void el.offsetWidth;
    return rect;
}

function rectOf(el) {
    const { left, top, right, bottom } = el.getBoundingClientRect();
    return { left, top, right, bottom };
}

function wireListeners(ctx, camera, navigation, recompute, remeasure) {
    const unsubCam = camera?.subscribe?.(recompute);
    const unsubAnchor = navigation?.subscribeAnchor?.(recompute);
    const ro = new ResizeObserver(remeasure);
    ro.observe(ctx.el);
    const mo = new MutationObserver(recompute);
    mo.observe(document.getElementById('world') || document.body, { childList: true, subtree: true });
    window.addEventListener('resize', remeasure);
    return () => {
        if (ctx.rafId) cancelAnimationFrame(ctx.rafId);
        if (ctx.hideTimer) clearTimeout(ctx.hideTimer);
        unsubCam?.();
        unsubAnchor?.();
        ro.disconnect();
        mo.disconnect();
        window.removeEventListener('resize', remeasure);
    };
}
