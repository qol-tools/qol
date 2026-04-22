import { useEffect } from 'preact/hooks';

const OCCLUSION_SELECTOR = '.world-view-slot';

function measureHomeRect(el) {
    const hadHide = el.style.getPropertyValue('--hide');
    const hideIsZero = hadHide === '' || hadHide === '0' || parseFloat(hadHide) === 0;
    if (hideIsZero) {
        const r = el.getBoundingClientRect();
        return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    }
    const hadTransition = el.style.getPropertyValue('transition');
    el.style.transition = 'none';
    el.style.setProperty('--hide', '0');
    void el.offsetWidth;
    const r = el.getBoundingClientRect();
    el.style.setProperty('--hide', hadHide);
    if (hadTransition !== '') el.style.transition = hadTransition;
    else el.style.removeProperty('transition');
    void el.offsetWidth;
    return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
}

function rectsOverlap(a, b) {
    return a.left < b.right
        && a.right > b.left
        && a.top < b.bottom
        && a.bottom > b.top;
}

export function useOverlayHide({ targetRef, camera, navigation, alwaysVisible = false }) {
    useEffect(() => {
        const el = targetRef?.current;
        if (!el) return undefined;
        if (alwaysVisible) {
            el.style.setProperty('--hide', '0');
            return undefined;
        }

        el.style.setProperty('--hide', '0');
        let home = null;
        let rafPending = 0;

        const measure = () => {
            home = measureHomeRect(el);
        };

        const recomputeNow = () => {
            if (!home) return;
            const slots = document.querySelectorAll(OCCLUSION_SELECTOR);
            let overlaps = false;
            for (const s of slots) {
                if (el.contains(s) || s.contains(el)) continue;
                const r = s.getBoundingClientRect();
                if (r.width === 0 || r.height === 0) continue;
                if (rectsOverlap(home, r)) { overlaps = true; break; }
            }
            el.style.setProperty('--hide', overlaps ? '1' : '0');
        };

        const recompute = () => {
            if (rafPending) return;
            rafPending = requestAnimationFrame(() => {
                rafPending = 0;
                recomputeNow();
            });
        };

        measure();
        requestAnimationFrame(recompute);

        const unsubCam = camera?.subscribe?.(recompute);
        const unsubAnchor = navigation?.subscribeAnchor?.(recompute);
        const ro = new ResizeObserver(() => { measure(); recompute(); });
        ro.observe(el);
        const onResize = () => { measure(); recompute(); };
        window.addEventListener('resize', onResize);
        const worldEl = document.getElementById('world') || document.body;
        const mo = new MutationObserver(recompute);
        mo.observe(worldEl, { childList: true, subtree: true });

        return () => {
            if (rafPending) cancelAnimationFrame(rafPending);
            unsubCam?.();
            unsubAnchor?.();
            ro.disconnect();
            mo.disconnect();
            window.removeEventListener('resize', onResize);
        };
    }, [targetRef, camera, navigation, alwaysVisible]);
}

export const useViewportHide = useOverlayHide;
