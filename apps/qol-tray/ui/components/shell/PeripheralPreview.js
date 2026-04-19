import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';

const NEIGHBOR_HARD_CAP = 4;
const ANIM_DURATION_MS = 240;
const ANIM_EASING = 'cubic-bezier(0.2, 0.8, 0.25, 1)';
const PARALLAX_PX = 28;

export function PeripheralPreview({ navigation, registry }) {
    const [, setTick] = useState(0);
    useEffect(() => {
        if (!navigation?.subscribeAnchor) return undefined;
        return navigation.subscribeAnchor(() => setTick((t) => t + 1));
    }, [navigation]);

    const traits = navigation?.getCurrentTraits?.() || {};
    const cfg = traits['peripheral-preview'];

    const slotRefs = useRef(new Map());
    const prevIdxRef = useRef(null);
    const animationsRef = useRef(new Map());

    useEffect(() => () => {
        for (const anim of animationsRef.current.values()) anim.cancel();
        animationsRef.current.clear();
    }, []);

    const anchorId = navigation?.getCurrentAnchor?.()?.pageId;
    const confinedPages = navigation?.getConfinedPages?.() || [];
    const idx = anchorId ? confinedPages.indexOf(anchorId) : -1;

    useLayoutEffect(() => {
        if (idx < 0) {
            prevIdxRef.current = null;
            return;
        }
        const prevIdx = prevIdxRef.current;
        prevIdxRef.current = idx;
        if (prevIdx == null || prevIdx === idx) return;

        const step = idx - prevIdx;
        if (!step) return;

        const reduced = typeof window !== 'undefined'
            && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
        if (reduced) return;

        const direction = step > 0 ? 1 : -1;
        for (const [key, el] of slotRefs.current) {
            if (!el || !el.isConnected) continue;
            const prior = animationsRef.current.get(key);
            const fromStyle = prior ? captureRunningState(el) : null;
            if (prior) prior.cancel();
            const anim = animateSlot(el, key, direction, fromStyle);
            if (!anim) continue;
            animationsRef.current.set(key, anim);
            anim.finished.then(
                () => { if (animationsRef.current.get(key) === anim) animationsRef.current.delete(key); },
                () => {},
            );
        }
    }, [idx, anchorId]);

    if (!cfg) return null;
    const requested = Number.isInteger(cfg.neighbors) ? cfg.neighbors : 1;
    if (requested <= 0) return null;
    const neighbors = Math.min(requested, NEIGHBOR_HARD_CAP);

    if (!anchorId || !confinedPages.length) return null;
    if (idx < 0) return null;

    const slots = [];
    for (let d = 1; d <= neighbors; d++) {
        const prevId = confinedPages[idx - d];
        slots.push({ id: prevId || null, side: 'prev', distance: d });
        const nextId = confinedPages[idx + d];
        slots.push({ id: nextId || null, side: 'next', distance: d });
    }

    return html`
        <div class="peripheral-preview" aria-hidden="true">
            ${slots.map((slot) => {
                const key = `${slot.side}-${slot.distance}`;
                return html`
                    <div
                        class=${`peripheral-slot peripheral-slot-${slot.side}${slot.id ? '' : ' peripheral-slot-empty'}`}
                        data-distance=${slot.distance}
                        key=${key}
                        ref=${(el) => {
                            if (el) slotRefs.current.set(key, el);
                            else slotRefs.current.delete(key);
                        }}
                    >
                        ${slot.id
                            ? html`<${PeripheralMini} registry=${registry} pageId=${slot.id} />`
                            : html`<div class="peripheral-edge"></div>`}
                    </div>
                `;
            })}
        </div>
    `;
}

function PeripheralMini({ registry, pageId }) {
    const entry = registry?.getEntry?.(pageId);
    const label = entry?.label || pageId;
    return html`
        <div class="peripheral-mini">
            <div class="peripheral-mini-label">${label}</div>
        </div>
    `;
}

const SCALE_FOR_DISTANCE = { 1: 0.8, 2: 0.6, 3: 0.4, 4: 0.28 };
const OPACITY_FOR_DISTANCE = { 1: 0.4, 2: 0.2, 3: 0.12, 4: 0.08 };

function captureRunningState(el) {
    const cs = getComputedStyle(el);
    return { transform: cs.transform, opacity: cs.opacity };
}

function animateSlot(el, key, direction, fromStyle) {
    const [side, distStr] = key.split('-');
    const distance = Number(distStr);
    const restScale = SCALE_FOR_DISTANCE[distance] ?? 0.2;
    const restOpacity = OPACITY_FOR_DISTANCE[distance] ?? 0.05;

    const sideSign = side === 'next' ? 1 : -1;
    const movingTowardThisSide = sideSign === direction;
    const fromScale = movingTowardThisSide
        ? (SCALE_FOR_DISTANCE[distance + 1] ?? Math.max(0.18, restScale - 0.18))
        : (SCALE_FOR_DISTANCE[Math.max(1, distance - 1)] ?? Math.min(0.95, restScale + 0.18));
    const fromOpacity = movingTowardThisSide
        ? (OPACITY_FOR_DISTANCE[distance + 1] ?? Math.max(0.04, restOpacity * 0.5))
        : (OPACITY_FOR_DISTANCE[Math.max(1, distance - 1)] ?? Math.min(1, restOpacity * 2));

    const parallaxX = direction * PARALLAX_PX * (1 / distance);

    const fromTransform = fromStyle?.transform && fromStyle.transform !== 'none'
        ? fromStyle.transform
        : `translate(${parallaxX}px, -50%) scale(${fromScale})`;
    const fromOpacityResolved = fromStyle?.opacity != null
        ? Number(fromStyle.opacity)
        : fromOpacity;
    const toTransform = `translate(0px, -50%) scale(${restScale})`;

    return el.animate(
        [
            { transform: fromTransform, opacity: fromOpacityResolved },
            { transform: toTransform, opacity: restOpacity },
        ],
        {
            duration: ANIM_DURATION_MS,
            easing: ANIM_EASING,
            fill: 'none',
        },
    );
}
